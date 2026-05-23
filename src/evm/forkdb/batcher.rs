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

use anyhow::{Result, anyhow};
use crossbeam::channel::{Receiver, RecvTimeoutError, Sender};
use serde_json::json;

use crate::evm::forkdb::cache::Cache;
use crate::evm::forkdb::dedup::DedupTable;
use crate::evm::forkdb::limiter::RateLimiter;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;
use crate::evm::forkdb::transport::Transport;

/// A queued request waiting for the background batcher to dispatch.
pub struct PendingRequest {
    pub request: Request,
    pub response_tx: Sender<Result<Response>>,
}

/// Background batcher that collects pending requests from a channel,
/// groups them into JSON-RPC batches, and dispatches responses back.
pub struct Batcher {
    pub request_rx: Receiver<PendingRequest>,
    pub transport: Box<dyn Transport>,
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

    fn process_batch(&self, batch: Vec<PendingRequest>) {
        if batch.is_empty() {
            return;
        }

        let is_single = batch.len() == 1;
        let payload = if is_single {
            let req = &batch[0].request;
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": req.method(),
                "params": req.params(),
            })
        } else {
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
        };

        // Rate limit gate: one HTTP POST == one token regardless of batch size.
        if let Some(ref limiter) = self.limiter {
            limiter.acquire();
        }

        // Live network fetch with exponential backoff retries.
        let value = 'retry: {
            let mut last_err = None;
            for attempt in 0..=self.retries {
                match self.transport.exec(&self.url, &payload) {
                    Ok(v) => {
                        let rpc_err = if is_single {
                            v.get("error").map(|e| format!("RPC error: {e}"))
                        } else {
                            v.as_array().and_then(|arr| {
                                arr.iter().find_map(|item| {
                                    item.get("error")
                                        .map(|e| format!("RPC error in batch: {e}"))
                                })
                            })
                        };
                        if let Some(err) = rpc_err {
                            last_err = Some(anyhow!(err));
                            if attempt < self.retries {
                                std::thread::sleep(self.backoff * 2_u32.pow(attempt));
                                continue;
                            }
                            break;
                        }
                        break 'retry v;
                    }
                    Err(e) => {
                        last_err = Some(e);
                        if attempt < self.retries {
                            std::thread::sleep(self.backoff * 2_u32.pow(attempt));
                        }
                    }
                }
            }
            let err = last_err.unwrap_or_else(|| anyhow!("RPC request failed"));
            for pending in batch {
                self.dedup
                    .complete(&pending.request.cache_key(), Err(anyhow!("{err}")));
                let _ = pending.response_tx.send(Err(anyhow!("{err}")));
            }
            return;
        };

        // Dispatch responses back to waiting threads.
        if is_single {
            let result = value
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let mut it = batch.into_iter();
            let Some(pending) = it.next() else {
                return;
            };
            let parsed = Response::parse(&pending.request, &result);
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
        } else {
            let arr = value.as_array().cloned().unwrap_or_default();
            let mut by_id: HashMap<usize, serde_json::Value> = HashMap::new();
            for mut item in arr {
                let Some(id) = item.get("id").and_then(|v| v.as_u64()).map(|v| v as usize) else {
                    continue;
                };
                if let Some(result) = item.as_object_mut().and_then(|obj| obj.remove("result")) {
                    by_id.insert(id, result);
                }
            }

            for (idx, pending) in batch.into_iter().enumerate() {
                let result = by_id.get(&idx).cloned().unwrap_or(serde_json::Value::Null);
                let parsed = Response::parse(&pending.request, &result);
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
        }
    }
}
