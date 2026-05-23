//! Background batcher that collects pending requests from a channel,
//! groups them into JSON-RPC batches, and dispatches responses back.
//!
//! Currently only one batcher thread is spawned per `Client`. All
//! fuzzer threads serialize every RPC request through this single
//! consumer.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use crossbeam::channel::{Receiver, RecvTimeoutError, Sender};
use serde_json::json;

use crate::evm::forkdb::cache::Cache;
use crate::evm::forkdb::dedup::DedupTable;
use crate::evm::forkdb::error::Error;
use crate::evm::forkdb::limiter::RateLimiter;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;
use crate::evm::forkdb::transport::Transport;

/// A queued request waiting for the background batcher to dispatch.
pub struct PendingRequest {
    pub request: Request,
    pub response_tx: Sender<std::result::Result<Response, Error>>,
}

/// Background batcher that collects pending requests from a channel,
/// groups them into JSON-RPC batches, and dispatches responses back.
pub struct Batcher {
    pub request_rx: Receiver<PendingRequest>,
    pub transport: Arc<dyn Transport>,
    pub url: String,
    pub retries: u32,
    pub backoff: Duration,
    pub batch_size: usize,
    pub batch_timeout: Duration,
    pub cache: Option<Arc<Cache>>,
    pub dedup: Arc<DedupTable>,
    pub limiter: Option<Arc<RateLimiter>>,
}

impl Batcher {
    pub fn run(&self) {
        while let Ok(first) = self.request_rx.recv() {
            let mut batch = vec![first];
            let deadline = Instant::now() + self.batch_timeout;

            while batch.len() < self.batch_size {
                match self.request_rx.recv_deadline(deadline) {
                    Ok(req) => batch.push(req),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        if !batch.is_empty() {
                            self.process_batch(batch);
                        }
                        return;
                    }
                }
            }

            self.process_batch(batch);
        }
    }

    fn build_payload(&self, batch: &[PendingRequest]) -> serde_json::Value {
        let array: Vec<serde_json::Value> = batch
            .iter()
            .enumerate()
            .map(|(idx, pending)| {
                json!({
                    "jsonrpc": "2.0",
                    "id": idx,
                    "method": pending.request.method(),
                    "params": pending.request.params(),
                })
            })
            .collect();
        json!(array)
    }

    fn dispatch_one(&self, pending: PendingRequest, result: &serde_json::Value) {
        let parsed = Response::parse(&pending.request, result).map_err(|e| Error::DecodeError {
            message: format!("{e}"),
        });
        if let Ok(resp) = &parsed
            && let Some(cache) = self.cache.as_ref()
        {
            cache.insert(&pending.request, resp.to_json());
        }
        let dedup_result = match &parsed {
            Ok(r) => Ok(r.to_json()),
            Err(e) => Err(anyhow!("{e}")),
        };
        self.dedup
            .complete(&pending.request.cache_key(), dedup_result);
        let _ = pending.response_tx.send(parsed);
    }

    fn dispatch_error(&self, pending: PendingRequest, err: &Error) {
        self.dedup
            .complete(&pending.request.cache_key(), Err(anyhow!("{err}")));
        let _ = pending.response_tx.send(Err(err.clone()));
    }

    fn sleep_duration(&self, attempt: u32) -> Duration {
        let max_backoff = Duration::from_millis(5_000);
        let multiplier = 2_u32.saturating_pow(attempt);
        self.backoff
            .checked_mul(multiplier)
            .map(|d| std::cmp::min(d, max_backoff))
            .unwrap_or(max_backoff)
    }

    fn process_batch(&self, mut batch: Vec<PendingRequest>) {
        if batch.is_empty() {
            return;
        }

        let mut payload = self.build_payload(&batch);

        // Rate limit gate: one HTTP POST == one token regardless of batch size.
        if let Some(ref limiter) = self.limiter {
            limiter.acquire();
        }

        // Live network fetch with exponential backoff retries.
        let mut last_err: Option<Error> = None;
        for attempt in 0..=self.retries {
            match self.transport.exec(&self.url, &payload) {
                Ok(v) => {
                    let mut by_id: HashMap<usize, serde_json::Value> = HashMap::new();

                    let arr: Vec<serde_json::Value> = if v.is_object() {
                        vec![v.clone()]
                    } else {
                        v.as_array().cloned().unwrap_or_default()
                    };
                    for mut item in arr {
                        let Some(id) = item.get("id").and_then(|v| v.as_u64()).map(|v| v as usize)
                        else {
                            continue;
                        };
                        if let Some(err) = item.get("error") {
                            let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
                            let message = err
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown RPC error")
                                .into();
                            last_err = Some(Error::RpcError { code, message });
                        } else if let Some(result) =
                            item.as_object_mut().and_then(|obj| obj.remove("result"))
                        {
                            by_id.insert(id, result);
                        }
                    }

                    let mut next_batch = Vec::new();
                    for (idx, pending) in batch.into_iter().enumerate() {
                        if let Some(result) = by_id.remove(&idx) {
                            self.dispatch_one(pending, &result);
                        } else {
                            next_batch.push(pending);
                        }
                    }

                    if next_batch.is_empty() {
                        return;
                    }

                    // Some items failed or were missing - retry only them.
                    batch = next_batch;
                    if attempt < self.retries {
                        payload = self.build_payload(&batch);
                        std::thread::sleep(self.sleep_duration(attempt));
                        continue;
                    }

                    // Retries exhausted: return errors for the remaining items.
                    let err = last_err.unwrap_or_else(|| Error::UnexpectedResponse {
                        message: "RPC request failed or response missing".into(),
                    });
                    for pending in batch {
                        self.dispatch_error(pending, &err);
                    }
                    return;
                }
                Err(e) => {
                    last_err = Some(Error::from_anyhow(e, &self.url));
                    if attempt < self.retries {
                        std::thread::sleep(self.sleep_duration(attempt));
                    }
                }
            }
        }

        let err = last_err.unwrap_or_else(|| Error::UnexpectedResponse {
            message: "RPC request failed".into(),
        });
        for pending in batch {
            self.dispatch_error(pending, &err);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::Result;
    use anyhow::anyhow;
    use crossbeam::channel::bounded;

    use serde_json::json;

    use crate::evm::forkdb::dedup::DedupTable;
    use crate::evm::forkdb::error::Error;
    use crate::evm::forkdb::request::Request;
    use crate::evm::forkdb::response::Response;
    use crate::evm::forkdb::transport::{MockTransport, Transport};

    use super::{Batcher, PendingRequest};

    /// Regression: retry backoff must be capped so a permanently down endpoint
    /// cannot stall the batcher (and all fuzzer threads) for unbounded time.
    #[test]
    fn backoff_is_capped() {
        let (_tx, rx) = bounded::<super::PendingRequest>(1);
        let batcher = Batcher {
            request_rx: rx,
            transport: Arc::new(MockTransport::default()),
            url: String::new(),
            retries: 10,
            backoff: Duration::from_millis(100),
            batch_size: 1,
            batch_timeout: Duration::from_millis(0),
            cache: None,
            dedup: Arc::new(DedupTable::new()),
            limiter: None,
        };

        let cap = Duration::from_millis(5_000);
        assert_eq!(batcher.sleep_duration(0), Duration::from_millis(100));
        assert_eq!(batcher.sleep_duration(1), Duration::from_millis(200));
        assert_eq!(batcher.sleep_duration(5), Duration::from_millis(3_200));
        assert_eq!(
            batcher.sleep_duration(10),
            cap,
            "backoff must be capped at 5 s to prevent unbounded growth"
        );
    }

    /// Regression: `sleep_duration` must not panic when `attempt >= 32`.
    /// `2_u32.pow(attempt)` overflows for `attempt >= 32`; we now use
    /// saturating math so any configurable `retries` value is safe.
    #[test]
    fn backoff_does_not_overflow() {
        let (_tx, rx) = bounded::<super::PendingRequest>(1);
        let batcher = Batcher {
            request_rx: rx,
            transport: Arc::new(MockTransport::default()),
            url: String::new(),
            retries: u32::MAX,
            backoff: Duration::from_millis(100),
            batch_size: 1,
            batch_timeout: Duration::from_millis(0),
            cache: None,
            dedup: Arc::new(DedupTable::new()),
            limiter: None,
        };

        let cap = Duration::from_millis(5_000);
        // attempt == 31 is the last value that fits in a u32 power-of-two.
        assert_eq!(batcher.sleep_duration(31), cap);
        // attempt >= 32 must not panic.
        assert_eq!(batcher.sleep_duration(32), cap);
        assert_eq!(batcher.sleep_duration(u32::MAX), cap);
    }

    #[derive(Debug)]
    struct ErrorTransport {
        message: String,
    }

    impl Transport for ErrorTransport {
        fn exec(&self, _url: &str, _payload: &serde_json::Value) -> Result<serde_json::Value> {
            Err(anyhow!("{}", self.message))
        }
    }

    /// Regression: when the transport returns a timeout, the batcher must
    /// preserve the endpoint URL in the error so callers with multiple RPC
    /// endpoints can identify which one failed.
    #[test]
    fn transport_timeout_preserves_url() {
        let (req_tx, req_rx) = bounded::<PendingRequest>(1);
        let url = "http://rpc.example";
        let batcher = Batcher {
            request_rx: req_rx,
            transport: Arc::new(ErrorTransport {
                message: "request timed out".into(),
            }),
            url: url.into(),
            retries: 0,
            backoff: Duration::from_millis(0),
            batch_size: 1,
            batch_timeout: Duration::from_millis(0),
            cache: None,
            dedup: Arc::new(DedupTable::new()),
            limiter: None,
        };

        let (resp_tx, resp_rx) = bounded(1);
        let request = Request::GetChainId { url_hash: 0 };
        req_tx
            .send(PendingRequest {
                request,
                response_tx: resp_tx,
            })
            .unwrap();
        drop(req_tx);

        batcher.run();

        let result = resp_rx.recv().unwrap();
        match result {
            Err(Error::RpcTimeout { url: err_url }) => {
                assert_eq!(err_url, url, "RpcTimeout must preserve the endpoint URL");
            }
            other => panic!("expected RpcTimeout with URL={url}, got {:?}", other),
        }
    }

    /// Regression: when the transport returns a rate-limit error, the batcher
    /// must preserve the endpoint URL in the error.
    #[test]
    fn transport_rate_limit_preserves_url() {
        let (req_tx, req_rx) = bounded::<PendingRequest>(1);
        let url = "http://rpc.example";
        let batcher = Batcher {
            request_rx: req_rx,
            transport: Arc::new(ErrorTransport {
                message: "429 too many requests".into(),
            }),
            url: url.into(),
            retries: 0,
            backoff: Duration::from_millis(0),
            batch_size: 1,
            batch_timeout: Duration::from_millis(0),
            cache: None,
            dedup: Arc::new(DedupTable::new()),
            limiter: None,
        };

        let (resp_tx, resp_rx) = bounded(1);
        let request = Request::GetChainId { url_hash: 0 };
        req_tx
            .send(PendingRequest {
                request,
                response_tx: resp_tx,
            })
            .unwrap();
        drop(req_tx);

        batcher.run();

        let result = resp_rx.recv().unwrap();
        match result {
            Err(Error::RateLimited { url: err_url }) => {
                assert_eq!(err_url, url, "RateLimited must preserve the endpoint URL");
            }
            other => panic!("expected RateLimited with URL={url}, got {:?}", other),
        }
    }

    #[derive(Debug)]
    struct SingleObjectTransport {
        response: serde_json::Value,
    }

    impl Transport for SingleObjectTransport {
        fn exec(&self, _url: &str, _payload: &serde_json::Value) -> Result<serde_json::Value> {
            Ok(self.response.clone())
        }
    }

    /// Regression: non-compliant RPC servers may return a single JSON object
    /// instead of a single-element array for a batch of one request.
    /// `process_batch` must accept both shapes.
    #[test]
    fn single_object_batch_response_is_accepted() {
        let (req_tx, req_rx) = bounded::<PendingRequest>(1);
        let batcher = Batcher {
            request_rx: req_rx,
            transport: Arc::new(SingleObjectTransport {
                response: json!({
                    "jsonrpc": "2.0",
                    "id": 0,
                    "result": "0x1",
                }),
            }),
            url: "http://rpc.example".into(),
            retries: 0,
            backoff: Duration::from_millis(0),
            batch_size: 1,
            batch_timeout: Duration::from_millis(0),
            cache: None,
            dedup: Arc::new(DedupTable::new()),
            limiter: None,
        };

        let (resp_tx, resp_rx) = bounded(1);
        let request = Request::GetChainId { url_hash: 0 };
        req_tx
            .send(PendingRequest {
                request,
                response_tx: resp_tx,
            })
            .unwrap();
        drop(req_tx);

        batcher.run();

        let result = resp_rx.recv().unwrap();
        assert!(
            matches!(result, Ok(Response::ChainId(1))),
            "single-object response for batch of one must be accepted, got {:?}",
            result
        );
    }
}
