//! RPC client with automatic batching, caching, deduplication, rate limiting,
//! and retries.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam::channel::{self, Sender};
use tracing::{instrument, trace};

use crate::evm::forkdb::batcher::{Batcher, PendingRequest};
use crate::evm::forkdb::cache::Cache;
use crate::evm::forkdb::config::Config;
use crate::evm::forkdb::dedup::DedupTable;
use crate::evm::forkdb::error::Error;
use crate::evm::forkdb::limiter::RateLimiter;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;
use crate::evm::forkdb::transport::Transport;

/// RPC client with automatic batching, two-layer caching, deduplication,
/// rate limiting, and retries.
///
/// Cloning is cheap (shares the same background worker and caches).
#[derive(Debug, Clone)]
pub struct Client {
    pub inner: Arc<ClientInner>,
}

#[derive(Debug)]
pub struct ClientInner {
    pub request_tx: Mutex<Sender<PendingRequest>>,
    pub cache: Option<Arc<Cache>>,
    pub dedup: Arc<DedupTable>,
}

impl Client {
    /// Create a client with the default HTTP transport (`ureq`).
    pub fn new(config: Config) -> Self {
        let agent_cfg = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(config.timeout_ms)))
            .build();
        let agent = ureq::Agent::new_with_config(agent_cfg);
        Self::new_with_transport(config, agent)
    }

    /// Create a client with a custom transport (e.g. [`MockTransport`] for
    /// testing).
    pub fn new_with_transport(config: Config, transport: impl Transport + 'static) -> Self {
        let limiter = config.rate_limit.map(|r| Arc::new(RateLimiter::new(r)));
        let cache = config.cache_dir.map(|d| Arc::new(Cache::new(d)));
        let dedup = Arc::new(DedupTable::new());
        let transport: Arc<dyn Transport> = Arc::new(transport);

        let batch_size = config.batch_size;
        let (request_tx, request_rx) = channel::bounded(batch_size.saturating_mul(2));

        let inner = Arc::new(ClientInner {
            request_tx: Mutex::new(request_tx),
            cache,
            dedup,
        });

        // Restart supervisor: if the batcher panics (e.g. ureq throws on
        // malformed JSON), respawn a fresh worker with a new channel so the
        // RPC subsystem stays alive for the rest of the campaign.
        std::thread::spawn({
            let inner = Arc::clone(&inner);
            let transport = Arc::clone(&transport);
            let url = config.url;
            let retries = config.retries;
            let backoff = Duration::from_millis(config.backoff_ms);
            let batch_timeout = Duration::from_millis(config.batch_timeout_ms);
            let limiter = limiter.clone();

            move || {
                let mut batcher = Batcher {
                    request_rx,
                    transport: Arc::clone(&transport),
                    url: url.clone(),
                    retries,
                    backoff,
                    batch_size,
                    batch_timeout,
                    cache: inner.cache.clone(),
                    dedup: Arc::clone(&inner.dedup),
                    limiter: limiter.clone(),
                };

                loop {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| batcher.run())) {
                        Ok(()) => break,
                        Err(_) => {
                            tracing::error!("batcher panicked, restarting");
                            let (tx, rx) = channel::bounded(batch_size.saturating_mul(2));
                            *inner.request_tx.lock().unwrap_or_else(|e| e.into_inner()) = tx;
                            let _old = std::mem::replace(&mut batcher.request_rx, rx);
                            drop(_old);
                        }
                    }
                }
            }
        });

        Self { inner }
    }

    /// Send one or more requests. They are automatically batched, deduplicated,
    /// rate-limited, retried, and cached.
    ///
    /// Even a single request goes through the batching worker so it benefits
    /// from the same caching, dedup, and retry logic as a full batch.
    #[instrument(skip(self), fields(count = reqs.len()))]
    pub fn request(&self, reqs: &[Request]) -> Result<Vec<Response>, Error> {
        if reqs.is_empty() {
            return Ok(Vec::new());
        }

        let mut results: Vec<Option<Response>> = vec![None; reqs.len()];
        let mut to_fetch: Vec<(usize, String, crate::evm::forkdb::dedup::DedupGuard)> = Vec::new();
        let mut any_err: Option<Error> = None;

        // 1. Check cache and dedup for each request
        for (idx, req) in reqs.iter().enumerate() {
            if let Some(cache) = self.inner.cache.as_ref()
                && let Some(value) = cache.get(req)
            {
                match Response::parse(req, &value) {
                    Ok(resp) => {
                        results[idx] = Some(resp);
                        continue;
                    }
                    Err(e) => trace!(?e, "cached parse failed, refetching"),
                }
            }

            let cache_key = req.cache_key();
            let mut skip_fetch = false;
            if let Some(result) = self.inner.dedup.register(&cache_key) {
                trace!("dedup hit for {}", cache_key);
                if let Err(e) = result {
                    any_err = Some(Error::from(e));
                    skip_fetch = true;
                } else if let Ok(v) = result
                    && let Ok(resp) = Response::parse(req, &v)
                {
                    results[idx] = Some(resp);
                    continue;
                }
            }
            if !skip_fetch {
                let guard = self.inner.dedup.guard(&cache_key);
                to_fetch.push((idx, cache_key, guard));
            }
        }

        if to_fetch.is_empty() {
            if let Some(e) = any_err {
                return Err(e);
            }
            return results
                .into_iter()
                .map(|o| {
                    o.ok_or_else(|| Error::UnexpectedResponse {
                        message: "missing response".into(),
                    })
                })
                .collect::<Result<Vec<Response>, Error>>();
        }

        // 2. Send all uncached / undeduped requests to the batching worker
        let mut receivers = Vec::with_capacity(to_fetch.len());
        for (idx, cache_key, guard) in to_fetch {
            let (tx, rx) = channel::bounded(1);
            self.inner
                .request_tx
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .send(PendingRequest {
                    request: reqs[idx].to_owned(),
                    response_tx: tx,
                })
                .map_err(|_| Error::Internal {
                    message: "batch worker shut down".into(),
                })?;
            receivers.push((idx, cache_key, guard, rx));
        }

        // 3. Collect all responses without failing early so every dedup
        //    guard gets properly completed even if one request errors.
        let mut resp_results = Vec::with_capacity(receivers.len());
        for (_, _, _, rx) in &receivers {
            resp_results.push(rx.recv().map_err(|_| Error::Internal {
                message: "batch worker response channel closed".into(),
            })?);
        }

        // 4. Fill results and deactivate dedup guards (batcher already cached & completed).
        for ((idx, _cache_key, guard, _), resp_result) in receivers.into_iter().zip(resp_results) {
            match resp_result {
                Ok(response) => {
                    guard.deactivate();
                    results[idx] = Some(response);
                }
                Err(e) => {
                    guard.deactivate();
                    if any_err.is_none() {
                        any_err = Some(e);
                    }
                }
            }
        }

        if let Some(e) = any_err {
            return Err(e);
        }

        results
            .into_iter()
            .map(|o| {
                o.ok_or_else(|| Error::UnexpectedResponse {
                    message: "missing response".into(),
                })
            })
            .collect::<Result<Vec<Response>, Error>>()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use alloy_primitives::Address;
    use anyhow::Result;
    use serde_json::json;
    use tempfile::tempdir;

    use crate::evm::forkdb::{Client, Config, MockTransport, Request, Response, Transport};

    /// Regression: the background batcher must be the sole thread that writes
    /// to the disk cache and completes dedup entries.  The caller thread
    /// (Client::request) must never perform these actions itself.
    #[test]
    fn client_caches_and_dedups_exactly_once() {
        let transport = MockTransport::default();
        let url = "mock://test";

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );

        let tmp = tempdir().unwrap();
        let config = Config::new(url).cache_dir(tmp.path()).batch_timeout_ms(0);
        let client = Client::new_with_transport(config, transport.clone());

        let reqs = &[Request::GetChainId];
        let res = client.request(reqs).unwrap();
        assert_eq!(res.len(), 1);

        let cache = client.inner.cache.as_ref().unwrap();
        assert_eq!(
            cache.insert_count.load(Ordering::SeqCst),
            1,
            "cache insert must happen exactly once (batcher only)"
        );

        let dedup = &client.inner.dedup;
        assert_eq!(
            dedup.complete_count.load(Ordering::SeqCst),
            1,
            "dedup complete must happen exactly once (batcher only)"
        );
    }

    /// Two threads requesting the same key concurrently must result in a single
    /// RPC call.  The second thread must block on the dedup table and receive
    /// the result via the batcher’s `complete` call, not issue a duplicate.
    #[test]
    fn client_dedups_concurrent_requests() {
        let transport = MockTransport::default();
        let url = "mock://test";

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );
        // Slow transport so both threads race while the request is in-flight.
        transport.set_delay(Duration::from_millis(50));

        let config = Config::new(url).batch_timeout_ms(0);
        let client = Arc::new(Client::new_with_transport(config, transport.clone()));

        let barrier = Arc::new(Barrier::new(2));
        let client2 = Arc::clone(&client);
        let barrier2 = Arc::clone(&barrier);

        let handle1 = std::thread::spawn(move || {
            barrier.wait();
            client.request(&[Request::GetChainId])
        });
        let handle2 = std::thread::spawn(move || {
            barrier2.wait();
            client2.request(&[Request::GetChainId])
        });

        let res1 = handle1.join().unwrap().unwrap();
        let res2 = handle2.join().unwrap().unwrap();

        assert_eq!(res1.len(), 1);
        assert_eq!(res2.len(), 1);
        assert_eq!(
            transport.call_count(url, &payload),
            1,
            "concurrent identical requests must result in exactly one RPC call"
        );
    }

    /// Regression: the request channel must be bounded so that producers block
    /// under backpressure instead of enqueueing unboundedly.
    #[test]
    fn request_channel_is_bounded() {
        let transport = MockTransport::default();
        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport);

        let tx = client.inner.request_tx.lock().unwrap();
        assert!(
            tx.capacity().is_some(),
            "request channel must be bounded, got unbounded"
        );
    }

    /// Regression: a transport panic must not kill the background batcher.
    /// The supervisor must respawn the worker so that subsequent requests are
    /// still serviced.
    #[test]
    fn batcher_recovers_from_transport_panic() {
        #[derive(Debug)]
        struct PanicTransport {
            panics_remaining: AtomicUsize,
        }

        impl Transport for PanicTransport {
            fn exec(&self, _url: &str, payload: &serde_json::Value) -> Result<serde_json::Value> {
                if self.panics_remaining.fetch_sub(1, Ordering::SeqCst) > 0 {
                    panic!("simulated transport panic");
                }
                // Return a valid response so the second request succeeds.
                let id = payload
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|item| item.get("id"))
                    .cloned()
                    .unwrap_or(json!(0));
                Ok(json!([{"jsonrpc":"2.0","id":id,"result":"0x1"}]))
            }
        }

        let transport = PanicTransport {
            panics_remaining: AtomicUsize::new(1),
        };

        let config = Config::new("mock://panic").batch_timeout_ms(0).retries(0);
        let client = Client::new_with_transport(config, transport);

        // First request triggers the panic inside the batcher.
        let res1 = client.request(&[Request::GetChainId]);
        assert!(
            res1.is_err(),
            "first request must fail because transport panicked and retries=0"
        );

        // Give the supervisor thread time to respawn.
        std::thread::sleep(Duration::from_millis(100));

        // Second request must succeed now that the batcher has restarted.
        let res2 = client.request(&[Request::GetChainId]);
        assert!(
            res2.is_ok(),
            "batcher must recover from transport panic; got: {:#}",
            res2.unwrap_err()
        );
        let responses = res2.unwrap();
        assert_eq!(responses.len(), 1);
        assert!(matches!(&responses[0], Response::ChainId(1)));
    }

    /// Regression: a JSON-RPC batch containing one error and one success must
    /// not fail the entire batch. The successful item should be dispatched
    /// immediately and only the failed item retried.
    #[test]
    fn batch_per_item_retry() {
        let transport = MockTransport::default();
        let url = "mock://test";

        let batch_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":["0x0000000000000000000000000000000000000000","0x1"]}
        ]);
        // First batch call: mixed response - first item succeeds, second errors.
        transport.mock_responses(
            url,
            &batch_payload,
            vec![json!([
                {"jsonrpc":"2.0","id":0,"result":"0x1"},
                {"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"bad block"}}
            ])],
        );

        let single_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0x0000000000000000000000000000000000000000","0x1"]}
        ]);
        // Retry of the failed item (now sent individually) succeeds.
        transport.mock_response(
            url,
            &single_payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x0"}]),
        );

        let config = Config::new(url)
            .batch_size(2)
            .batch_timeout_ms(100)
            .retries(1)
            .backoff_ms(0);
        let client = Client::new_with_transport(config, transport.clone());

        let reqs = &[
            Request::GetChainId,
            Request::GetBalance {
                address: Address::ZERO,
                block: 1,
            },
        ];
        let res = client.request(reqs).unwrap();
        assert_eq!(res.len(), 2);
        assert!(matches!(&res[0], Response::ChainId(1)));
        assert!(matches!(&res[1], Response::Balance(v) if v.is_zero()));

        // The full batch must have been submitted exactly once.
        assert_eq!(
            transport.call_count(url, &batch_payload),
            1,
            "full batch should be sent once"
        );
        // Only the failed item should have been retried individually.
        assert_eq!(
            transport.call_count(url, &single_payload),
            1,
            "failed item should be retried individually"
        );
    }

    /// Regression: when an in-flight request fails after all batcher retries are
    /// exhausted, waiters must receive the error and propagate it. They must NOT
    /// treat the dedup error as a cache miss and resubmit the request.
    #[test]
    fn client_dedup_waiter_does_not_resubmit_on_error() {
        let transport = MockTransport::default();
        let url = "mock://test";

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        // Intentionally do NOT register a mock response, so the transport
        // always returns an error (simulating a down endpoint).
        transport.set_delay(Duration::from_millis(100));

        let config = Config::new(url)
            .batch_timeout_ms(0)
            .retries(0) // No retries - fail immediately.
            .backoff_ms(0);
        let client = Arc::new(Client::new_with_transport(config, transport.clone()));

        let barrier = Arc::new(Barrier::new(2));
        let client2 = Arc::clone(&client);
        let barrier2 = Arc::clone(&barrier);

        let handle1 = std::thread::spawn(move || {
            barrier.wait();
            client.request(&[Request::GetChainId])
        });
        let handle2 = std::thread::spawn(move || {
            barrier2.wait();
            client2.request(&[Request::GetChainId])
        });

        let res1 = handle1.join().unwrap();
        let res2 = handle2.join().unwrap();

        // Both should error.
        assert!(res1.is_err(), "leader must get RPC error");
        assert!(res2.is_err(), "waiter must get RPC error, not resubmit");

        // Only ONE RPC call should have been made. With the bug, the waiter
        // would resubmit after receiving the dedup error, causing a second call.
        assert_eq!(
            transport.call_count(url, &payload),
            1,
            "waiter must not resubmit after dedup error; only the leader's request should hit the transport"
        );
    }
}
