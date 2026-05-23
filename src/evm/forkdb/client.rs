//! RPC client with automatic batching, caching, deduplication, rate limiting,
//! and retries.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use crossbeam::channel::{self, Sender};
use tracing::{instrument, trace};

use crate::evm::forkdb::batcher::{Batcher, PendingRequest};
use crate::evm::forkdb::cache::Cache;
use crate::evm::forkdb::config::Config;
use crate::evm::forkdb::dedup::DedupTable;
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
    pub request_tx: Sender<PendingRequest>,
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

        let (request_tx, request_rx) = channel::unbounded();

        let batcher = Batcher {
            request_rx,
            transport: Box::new(transport),
            url: config.url,
            retries: config.retries,
            backoff: Duration::from_millis(config.backoff_ms),
            batch_size: config.batch_size,
            batch_timeout: Duration::from_millis(config.batch_timeout_ms),
            cache: cache.clone(),
            dedup: dedup.clone(),
            limiter: limiter.clone(),
        };

        std::thread::spawn(move || batcher.run());

        Self {
            inner: Arc::new(ClientInner {
                request_tx,
                cache,
                dedup,
            }),
        }
    }

    /// Send one or more requests. They are automatically batched, deduplicated,
    /// rate-limited, retried, and cached.
    ///
    /// Even a single request goes through the batching worker so it benefits
    /// from the same caching, dedup, and retry logic as a full batch.
    #[instrument(skip(self), fields(count = reqs.len()))]
    pub fn request(&self, reqs: &[Request]) -> Result<Vec<Response>> {
        if reqs.is_empty() {
            return Ok(Vec::new());
        }

        let mut results: Vec<Option<Response>> = vec![None; reqs.len()];
        let mut to_fetch: Vec<(usize, String, crate::evm::forkdb::dedup::DedupGuard)> = Vec::new();

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
            if let Some(result) = self.inner.dedup.register(&cache_key) {
                trace!("dedup hit for {}", cache_key);
                if let Ok(resp) = result.and_then(|v| Response::parse(req, &v)) {
                    results[idx] = Some(resp);
                    continue;
                }
            }

            let guard = self.inner.dedup.guard(&cache_key);
            to_fetch.push((idx, cache_key, guard));
        }

        if to_fetch.is_empty() {
            return results
                .into_iter()
                .map(|o| o.context("missing response"))
                .collect::<Result<Vec<Response>>>();
        }

        // 2. Send all uncached / undeduped requests to the batching worker
        let mut receivers = Vec::with_capacity(to_fetch.len());
        for (idx, cache_key, guard) in to_fetch {
            let (tx, rx) = channel::bounded(1);
            self.inner
                .request_tx
                .send(PendingRequest {
                    request: reqs[idx].to_owned(),
                    response_tx: tx,
                })
                .map_err(|_| anyhow!("batch worker shut down"))?;
            receivers.push((idx, cache_key, guard, rx));
        }

        // 3. Collect all responses without failing early so every dedup
        //    guard gets properly completed even if one request errors.
        let mut resp_results = Vec::with_capacity(receivers.len());
        for (_, _, _, rx) in &receivers {
            resp_results.push(
                rx.recv()
                    .map_err(|_| anyhow!("batch worker response channel closed"))?,
            );
        }

        // 4. Fill results and deactivate dedup guards (batcher already cached & completed).
        let mut any_err = None;
        for ((idx, _cache_key, guard, _), resp_result) in receivers.into_iter().zip(resp_results) {
            match resp_result {
                Ok(response) => {
                    guard.deactivate();
                    results[idx] = Some(response);
                }
                Err(e) => {
                    guard.deactivate();
                    any_err = Some(e);
                }
            }
        }

        if let Some(e) = any_err {
            return Err(e);
        }

        results
            .into_iter()
            .map(|o| o.context("missing response"))
            .collect::<Result<Vec<Response>>>()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::evm::forkdb::{Client, Config, MockTransport, Request};

    /// Regression: the background batcher must be the sole thread that writes
    /// to the disk cache and completes dedup entries.  The caller thread
    /// (Client::request) must never perform these actions itself.
    #[test]
    fn client_caches_and_dedups_exactly_once() {
        let transport = MockTransport::default();
        let url = "mock://test";

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_chainId",
            "params": [],
        });
        transport.mock_response(
            url,
            &payload,
            json!({"jsonrpc":"2.0","id":1,"result":"0x1"}),
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

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_chainId",
            "params": [],
        });
        transport.mock_response(
            url,
            &payload,
            json!({"jsonrpc":"2.0","id":1,"result":"0x1"}),
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
}
