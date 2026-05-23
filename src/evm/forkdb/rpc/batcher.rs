//! Automatic request batching backed by a background thread.
//!
//! Individual requests are submitted via channels and flushed as JSON-RPC
//! batches when either `batch_size` is reached or `batch_timeout` expires.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde_json::json;
use tracing::trace;

use crate::evm::forkdb::rpc::limiter::RateLimiter;
use crate::evm::forkdb::rpc::transport::Transport;
use crate::evm::forkdb::rpc::types::RpcRequest;

struct BatchedRequest {
    id: u64,
    payload: serde_json::Value,
    response_tx: Sender<Result<serde_json::Value>>,
    arrived: Instant,
}

/// Background batch collector.
///
/// Dispatches individual requests as JSON-RPC batches when either
/// `batch_size` is reached or `batch_timeout` expires.
#[derive(Debug, Clone)]
pub struct Batcher {
    sender: Sender<BatchedRequest>,
}

impl Batcher {
    pub fn new(
        transport: Arc<dyn Transport>,
        url: impl Into<String>,
        retries: u32,
        backoff: Duration,
        limiter: Option<Arc<RateLimiter>>,
        batch_size: usize,
        batch_timeout: Duration,
    ) -> Self {
        let url = url.into();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            run_batch_loop(
                rx,
                transport,
                &url,
                retries,
                backoff,
                limiter,
                batch_size,
                batch_timeout,
            );
        });
        Self { sender: tx }
    }

    /// Submit a single request and receive a [`Receiver`] for the response.
    pub fn submit(&self, id: u64, request: &RpcRequest) -> Receiver<Result<serde_json::Value>> {
        let (tx, rx) = channel();
        let req = BatchedRequest {
            id,
            payload: request.to_json_payload(id),
            response_tx: tx,
            arrived: Instant::now(),
        };
        let _ = self.sender.send(req);
        rx
    }
}

#[allow(clippy::too_many_arguments)]
fn run_batch_loop(
    rx: Receiver<BatchedRequest>,
    transport: Arc<dyn Transport>,
    url: &str,
    retries: u32,
    backoff: Duration,
    limiter: Option<Arc<RateLimiter>>,
    batch_size: usize,
    batch_timeout: Duration,
) {
    let mut pending: Vec<BatchedRequest> = Vec::new();

    loop {
        // Flush if batch size reached
        if pending.len() >= batch_size {
            flush_batch(&mut pending, &transport, url, retries, backoff, &limiter);
            continue;
        }

        if pending.is_empty() {
            match rx.recv() {
                Ok(req) => {
                    pending.push(req);
                    continue;
                }
                Err(_) => break,
            }
        }

        let elapsed = pending
            .first()
            .map(|r| r.arrived.elapsed())
            .unwrap_or(Duration::ZERO);
        let remaining = batch_timeout.saturating_sub(elapsed);
        if remaining.is_zero() {
            flush_batch(&mut pending, &transport, url, retries, backoff, &limiter);
            continue;
        }

        match rx.recv_timeout(remaining) {
            Ok(req) => pending.push(req),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                flush_batch(&mut pending, &transport, url, retries, backoff, &limiter);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if !pending.is_empty() {
                    flush_batch(&mut pending, &transport, url, retries, backoff, &limiter);
                }
                break;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn flush_batch(
    pending: &mut Vec<BatchedRequest>,
    transport: &Arc<dyn Transport>,
    url: &str,
    retries: u32,
    backoff: Duration,
    limiter: &Option<Arc<RateLimiter>>,
) {
    if pending.is_empty() {
        return;
    }

    trace!(batch_len = pending.len(), url, "flushing batch");

    // Rate limit: one token per HTTP POST regardless of inner request count.
    if let Some(ref l) = *limiter {
        l.acquire();
    }

    let mut requests: Vec<BatchedRequest> = std::mem::take(pending);
    let mut payloads = Vec::with_capacity(requests.len());
    for req in &mut requests {
        payloads.push(std::mem::take(&mut req.payload));
    }
    let batch_payload = if payloads.len() == 1 {
        payloads.remove(0)
    } else {
        serde_json::Value::Array(payloads)
    };

    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..=retries {
        match transport.exec(url, &batch_payload) {
            Ok(envelope) => {
                // Check for RPC errors
                let rpc_error = match &envelope {
                    serde_json::Value::Array(arr) => arr
                        .iter()
                        .find_map(|item| item.get("error"))
                        .map(|e| format!("RPC error in batch response: {e}")),
                    serde_json::Value::Object(obj) => {
                        obj.get("error").map(|e| format!("RPC error: {e}"))
                    }
                    _ => Some("invalid RPC response".into()),
                };

                if let Some(err_msg) = rpc_error {
                    last_err = Some(anyhow!("{err_msg}"));
                    if attempt < retries {
                        std::thread::sleep(backoff * 2_u32.pow(attempt));
                        continue;
                    }
                    // All retries exhausted - send error to every waiter
                    let err = last_err.unwrap_or_else(|| anyhow!("RPC request failed"));
                    for req in requests {
                        let _ = req.response_tx.send(Err(anyhow!("{err}")));
                    }
                    return;
                }

                // Distribute responses by matching JSON-RPC id
                let mut by_id = HashMap::new();
                match envelope {
                    serde_json::Value::Array(arr) => {
                        for item in arr {
                            if let Some(id) = item.get("id").and_then(|v| v.as_u64()) {
                                by_id.insert(id, item);
                            }
                        }
                    }
                    obj => {
                        if let Some(id) = obj.get("id").and_then(|v| v.as_u64()) {
                            by_id.insert(id, obj);
                        }
                    }
                }

                for req in requests {
                    let response = by_id.remove(&req.id).unwrap_or_else(|| {
                        json!({
                            "jsonrpc": "2.0",
                            "id": req.id,
                            "error": "missing response in batch"
                        })
                    });
                    let _ = req.response_tx.send(Ok(response));
                }
                return;
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < retries {
                    std::thread::sleep(backoff * 2_u32.pow(attempt));
                }
            }
        }
    }

    let err = last_err.unwrap_or_else(|| anyhow!("RPC request failed"));
    for req in requests {
        let _ = req.response_tx.send(Err(anyhow!("{err}")));
    }
}
