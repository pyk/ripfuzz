//! Background batcher that collects pending requests from a channel,
//! groups them into JSON-RPC batches, and dispatches responses back.
//!
//! # Known Limitations
//!
//! 1. Single Batcher Thread. Currently only one batcher thread is spawned per
//!    `Client`. All fuzzer threads serialize every RPC request through this
//!    single consumer.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use crossbeam::channel::{Receiver, RecvTimeoutError, Sender};
use serde_json::json;

use crate::evm::forkdb::cache::Cache;
use crate::evm::forkdb::db::ForkDBError;
use crate::evm::forkdb::dedup::DedupTable;
use crate::evm::forkdb::limiter::RateLimiter;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;
use crate::evm::forkdb::transport::Transport;

/// A queued request waiting for the background batcher to dispatch.
pub struct PendingRequest {
    pub request: Request,
    pub response_tx: Sender<std::result::Result<Response, ForkDBError>>,
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
        let parsed =
            Response::parse(&pending.request, result).map_err(|e| ForkDBError::DecodeError {
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

    fn dispatch_error(&self, pending: PendingRequest, err: &ForkDBError) {
        self.dedup
            .complete(&pending.request.cache_key(), Err(anyhow!("{err}")));
        let _ = pending.response_tx.send(Err(err.clone()));
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
        let mut last_err: Option<ForkDBError> = None;
        for attempt in 0..=self.retries {
            match self.transport.exec(&self.url, &payload) {
                Ok(v) => {
                    let mut by_id: HashMap<usize, serde_json::Value> = HashMap::new();

                    let arr = v.as_array().cloned().unwrap_or_default();
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
                            last_err = Some(ForkDBError::RpcError { code, message });
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
                        std::thread::sleep(self.backoff * 2_u32.pow(attempt));
                        continue;
                    }

                    // Retries exhausted: return errors for the remaining items.
                    let err = last_err.unwrap_or_else(|| ForkDBError::UnexpectedResponse {
                        message: "RPC request failed or response missing".into(),
                    });
                    for pending in batch {
                        self.dispatch_error(pending, &err);
                    }
                    return;
                }
                Err(e) => {
                    last_err = Some(ForkDBError::from(e));
                    if attempt < self.retries {
                        std::thread::sleep(self.backoff * 2_u32.pow(attempt));
                    }
                }
            }
        }

        let err = last_err.unwrap_or_else(|| ForkDBError::UnexpectedResponse {
            message: "RPC request failed".into(),
        });
        for pending in batch {
            self.dispatch_error(pending, &err);
        }
    }
}
