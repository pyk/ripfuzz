//! SharedBackend: lock-free global cache + batched RPC fetcher.
//!
//! ## Design Goal
//!
//! Reduce the number of HTTP round-trips during fuzzing by letting the fuzzer
//! threads themselves batch and deduplicate RPC requests. There is **no**
//! background worker thread.
//!
//! ## SharedBackend Initialization Phase
//!
//! When a [`SharedBackend`] is created it pre-populates a lock-free
//! *global cache* from the on-disk cache directory. Every fuzzer thread sees
//! the same map, so a value cached by one thread is instantly visible to all
//! others.
//!
//! ## SharedBackend Execution Phase
//!
//! A fuzzer thread calls [`SharedBackend::fetch_or_wait`] with a slice of
//! [`Request`]s.
//!
//! 1. **Fast path**: every request is already in `global_cache`. The function
//!    returns the parsed [`Response`]s immediately without locking.
//!
//! 2. **Slow path**: at least one request is missing. The thread acquires the
//!    global `batch_state` mutex, adds its missing requests to the pending set
//!    (deduplicated by `cache_key`), and either:
//!    * becomes the **fetcher** when `batch_size` is reached or the batch
//!      deadline (default 50 ms) has expired; or
//!    * releases the mutex and blocks on a `Condvar` while waiting for a
//!      fetcher to complete.
//!
//!    The fetcher takes ownership of the pending slice, drops the mutex,
//!    issues a single JSON-RPC batch via `ureq`, retries on transient
//!    transport errors, and either inserts all responses into `global_cache`
//!    (and writes them to disk) or publishes a single error back into
//!    `batch_state` for every key in the batch.  Finally it wakes all
//!    waiting threads with `notify_all`.
//!
//! 3. Waiters wake up, re-check `global_cache`, and return.  If a previous
//!    batch produced an error for a key, that error is returned immediately.
//!    When a key is re-submitted to a new batch its stale error is cleared so
//!    the key is retried.
//!
//! ## Thread Roles
//!
//! * **Waiter** – adds requests to the batch and sleeps on the condvar.
//! * **Fetcher** – the first thread that fills the buffer or hits the
//!   timeout.  It runs the HTTP call while other threads continue adding to
//!   the *next* batch.
//!
//! Only one thread can be the fetcher for a given batch, but while the
//! fetcher performs I/O (mutex released) another thread may become the
//! fetcher for the subsequent batch.
//!
//! ## Batch Processing Specification
//!
//! A batch is a **single HTTP POST request** containing all pending JSON-RPC
//! items.  The request is retried as a whole; there is no per-item retry.
//!
//! ### Retry conditions
//!
//! The batch is retried (with capped exponential backoff) when the
//! transport returns:
//!
//! * HTTP timeout
//! * Connection error
//! * HTTP 429 / 503 / 504
//!
//! ### Failure conditions
//!
//! If the transport succeeds but the response body is invalid JSON, or the
//! response array length does not match the number of requests, the batch
//! fails immediately and the error is returned to every waiting thread.
//!
//! If an individual JSON-RPC item in the response contains an `"error"`
//! object, or if `eth_getBlockByNumber` returns `"result": null` for a
//! missing block, the entire batch fails and the error is returned to all
//! waiting threads.
//!
//! ### Success
//!
//! On success every item is inserted into the lock-free `global_cache`
//! (visible to all threads immediately) and written to disk **outside**
//! the mutex.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use papaya::HashMap as PapayaMap;
use parking_lot::{Condvar, Mutex};
use serde_json::Value;
use tracing::instrument;
use walkdir::WalkDir;

use crate::evm::forkdb::config::Config;
use crate::evm::forkdb::error::Error;
use crate::evm::forkdb::limiter::RateLimiter;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;
use crate::evm::forkdb::transport::Transport;

/// Shared RPC backend with automatic batching, caching, and deduplication.
///
/// Cloning is cheap (shares the same inner state).
#[derive(Debug, Clone)]
pub struct SharedBackend {
    inner: Arc<SharedBackendInner>,
}

#[derive(Debug)]
struct SharedBackendInner {
    /// Lock-free global cache shared by all fuzzer threads.
    global_cache: PapayaMap<String, Value>,
    /// Coordinates pending batches and unclaimed errors.
    batch_state: Mutex<BatchState>,
    batch_condvar: Condvar,
    batch_size: usize,
    batch_timeout: Duration,
    transport: Arc<dyn Transport>,
    url: String,
    retries: u32,
    backoff: Duration,
    limiter: Option<Arc<RateLimiter>>,
    cache_dir: Option<PathBuf>,
}

#[derive(Debug)]
struct BatchState {
    pending: Vec<Request>,
    keys: HashSet<String>,
    deadline: Option<Instant>,
    /// Error from the most recent failed batch (shared by all keys in that batch).
    last_error: Option<Error>,
    /// Keys that were in the most recent batch so waiters can tell whether
    /// the failure applies to them.
    last_batch_keys: HashSet<String>,
    /// `true` while a thread is performing an HTTP call so that other
    /// threads wait instead of spawning a duplicate fetcher.
    fetcher_in_flight: bool,
}

impl SharedBackend {
    /// Create a backend with the default HTTP transport (`ureq`).
    pub fn new(config: Config) -> Self {
        let agent_cfg = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(config.timeout_ms)))
            .build();
        let agent = ureq::Agent::new_with_config(agent_cfg);
        Self::new_with_transport(config, agent)
    }

    /// Create a backend with a custom transport (e.g. [`MockTransport`] for
    /// testing).
    pub fn new_with_transport(config: Config, transport: impl Transport + 'static) -> Self {
        let limiter = config.rate_limit.map(|r| Arc::new(RateLimiter::new(r)));
        let transport: Arc<dyn Transport> = Arc::new(transport);

        let global_cache = PapayaMap::new();
        if let Some(ref dir) = config.cache_dir {
            load_disk_cache(dir, &global_cache);
        }

        let inner = Arc::new(SharedBackendInner {
            global_cache,
            batch_state: Mutex::new(BatchState {
                pending: Vec::new(),
                keys: HashSet::new(),
                deadline: None,
                last_error: None,
                last_batch_keys: HashSet::new(),
                fetcher_in_flight: false,
            }),
            batch_condvar: Condvar::new(),
            batch_size: config.batch_size,
            batch_timeout: Duration::from_millis(config.batch_timeout_ms),
            transport,
            url: config.url,
            retries: config.retries,
            backoff: Duration::from_millis(config.backoff_ms),
            limiter,
            cache_dir: config.cache_dir,
        });

        Self { inner }
    }

    /// Fetch one or more requests.
    ///
    /// Requests are automatically batched, deduplicated, rate-limited,
    /// retried, and cached.
    #[instrument(skip(self), fields(count = reqs.len()))]
    pub fn fetch_or_wait(&self, reqs: &[Request]) -> Result<Vec<Response>, Error> {
        if reqs.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(results) = self.try_fast_path(reqs)? {
            return Ok(results);
        }
        self.try_slow_path(reqs)
    }

    // ------------------------------------------------------------------
    // Fast path
    // ------------------------------------------------------------------

    /// Try to satisfy every request from the lock-free global cache.
    ///
    /// Returns `Ok(Some(_))` when every key is present and parses cleanly.
    /// Returns `Ok(None)` as soon as the first missing key is found.
    fn try_fast_path(&self, reqs: &[Request]) -> Result<Option<Vec<Response>>, Error> {
        let map = self.inner.global_cache.pin();
        let mut results = Vec::with_capacity(reqs.len());
        for req in reqs {
            match map.get(&req.cache_key()) {
                Some(value) => results.push(Response::parse(req, value)?),
                None => return Ok(None),
            }
        }
        Ok(Some(results))
    }

    // ------------------------------------------------------------------
    // Slow path
    // ------------------------------------------------------------------

    /// Coordinate with other threads until every request is resolved.
    ///
    /// The loop repeats until all of the caller's requests are resolved.
    /// A typical caller enqueues once, either fetches or waits, and then
    /// loops back after waking to collect results or become the next fetcher.
    fn try_slow_path(&self, reqs: &[Request]) -> Result<Vec<Response>, Error> {
        let mut state = self.inner.batch_state.lock();
        loop {
            // --- Check global cache ---
            let map = self.inner.global_cache.pin();
            let mut results = Vec::with_capacity(reqs.len());
            let mut missing = Vec::new();

            for req in reqs {
                let key = req.cache_key();
                if let Some(value) = map.get(&key) {
                    results.push(Response::parse(req, value)?);
                } else {
                    missing.push(key);
                }
            }

            if missing.is_empty() {
                return Ok(results);
            }

            // --- Clear stale errors for keys we are about to retry ---
            for k in &missing {
                state.last_batch_keys.remove(k);
            }
            if state.last_batch_keys.is_empty() {
                state.last_error = None;
            }

            // --- Check whether the most recent batch still failed for any of our keys ---
            if let Some(ref err) = state.last_error
                && missing.iter().any(|k| state.last_batch_keys.contains(k))
            {
                return Err(err.clone());
            }

            // --- Enqueue missing requests ---
            for req in reqs {
                let key = req.cache_key();
                if state.keys.insert(key) {
                    state.pending.push(req.clone());
                }
            }
            if state.deadline.is_none() && !state.pending.is_empty() {
                state.deadline = Some(Instant::now() + self.inner.batch_timeout);
            }

            // --- Decide: fetch or wait ---
            let size_hit = state.pending.len() >= self.inner.batch_size;
            let deadline_hit = match state.deadline {
                Some(d) => Instant::now() >= d,
                None => false,
            };
            if (size_hit || deadline_hit) && !state.fetcher_in_flight {
                // Fetcher role – runs the batch, notifies waiters, and returns directly.
                state.fetcher_in_flight = true;
                let batch = std::mem::take(&mut state.pending);
                state.last_batch_keys = std::mem::take(&mut state.keys);
                state.deadline = None;
                state.last_error = None;
                drop(state);

                match self.execute_batch(batch) {
                    Ok(successes) => {
                        let map = self.inner.global_cache.pin();
                        for (key, value) in successes {
                            if let Some(ref dir) = self.inner.cache_dir {
                                let _ = write_disk_cache(dir, &key, &value);
                            }
                            map.insert(key, value);
                        }

                        state = self.inner.batch_state.lock();
                        state.fetcher_in_flight = false;
                        self.inner.batch_condvar.notify_all();

                        // Return own results directly from the now-populated cache.
                        let map = self.inner.global_cache.pin();
                        let mut results = Vec::with_capacity(reqs.len());
                        for req in reqs {
                            let key = req.cache_key();
                            let Some(value) = map.get(&key) else {
                                return Err(Error::Internal {
                                    message: "fetcher inserted all keys".into(),
                                });
                            };
                            results.push(Response::parse(req, value)?);
                        }
                        return Ok(results);
                    }
                    Err(err) => {
                        state = self.inner.batch_state.lock();
                        state.last_error = Some(err.clone());
                        state.fetcher_in_flight = false;
                        self.inner.batch_condvar.notify_all();
                        return Err(err);
                    }
                }
            }

            // Waiter role – sleeps until a fetcher finishes or the deadline expires.
            let timeout = state.deadline.map_or(self.inner.batch_timeout, |d| {
                d.saturating_duration_since(Instant::now())
            });
            self.inner.batch_condvar.wait_for(&mut state, timeout);
            // After waking the thread loops back to collect results,
            // receive the batch error, or become the next fetcher.
        }
    }

    // ------------------------------------------------------------------
    // Batch execution (pure: no shared mutable state)
    // ------------------------------------------------------------------

    /// Execute a JSON-RPC batch.
    ///
    /// The batch is sent as a single HTTP POST.  If the transport returns a
    /// transient error (timeout, connection failure, HTTP 429/503/504) the
    /// whole batch is retried with capped exponential backoff.  If the
    /// response is invalid JSON, the array length does not match, or any
    /// individual item contains an error object, the entire batch fails
    /// immediately and the error is returned to the caller.
    fn execute_batch(&self, batch: Vec<Request>) -> Result<HashMap<String, Value>, Error> {
        let deduped = Self::deduplicate_requests(batch);
        let payload = build_payload(&deduped);

        // Rate limit gate: one HTTP POST == one token regardless of batch size.
        if let Some(ref limiter) = self.inner.limiter {
            limiter.acquire();
        }

        for attempt in 0..=self.inner.retries {
            match self.inner.transport.exec(&self.inner.url, &payload) {
                Ok(response) => return Self::parse_batch_response(response, deduped),
                Err(e) => {
                    let err = Error::from_anyhow(e, &self.inner.url);
                    if !err.is_transient() || attempt >= self.inner.retries {
                        return Err(err);
                    }
                    std::thread::sleep(self.sleep_duration(attempt));
                }
            }
        }

        unreachable!("loop always returns")
    }

    /// Remove duplicate `cache_key`s from a batch, preserving order.
    fn deduplicate_requests(batch: Vec<Request>) -> Vec<Request> {
        let mut out = Vec::with_capacity(batch.len());
        let mut seen = HashSet::new();
        for req in batch {
            let key = req.cache_key();
            if seen.insert(key) {
                out.push(req);
            }
        }
        out
    }

    /// Parse a JSON-RPC response and validate that every request has a
    /// matching, successful result.
    fn parse_batch_response(
        response: Value,
        deduped: Vec<Request>,
    ) -> Result<HashMap<String, Value>, Error> {
        let arr: Vec<Value> = if response.is_object() && deduped.len() == 1 {
            vec![response]
        } else {
            response
                .as_array()
                .cloned()
                .ok_or_else(|| Error::UnexpectedResponse {
                    message: "expected JSON array response for batch request".into(),
                })?
        };

        if arr.len() != deduped.len() {
            return Err(Error::UnexpectedResponse {
                message: format!("expected {} responses, got {}", deduped.len(), arr.len()),
            });
        }

        let mut by_id: HashMap<usize, Value> = HashMap::with_capacity(arr.len());
        for mut item in arr {
            let Some(id) = item.get("id").and_then(|v| v.as_u64()).map(|v| v as usize) else {
                return Err(Error::UnexpectedResponse {
                    message: "missing id in JSON-RPC response item".into(),
                });
            };
            if item.get("error").is_some() {
                return Err(Error::UnexpectedResponse {
                    message: "JSON-RPC response contains error object".into(),
                });
            }
            let result = item
                .as_object_mut()
                .and_then(|obj| obj.remove("result"))
                .ok_or_else(|| Error::UnexpectedResponse {
                    message: "missing result in JSON-RPC response item".into(),
                })?;
            by_id.insert(id, result);
        }

        let mut successes = HashMap::with_capacity(deduped.len());
        for (idx, req) in deduped.into_iter().enumerate() {
            let result = by_id
                .remove(&idx)
                .ok_or_else(|| Error::UnexpectedResponse {
                    message: format!("missing response for request id {idx}"),
                })?;
            if result.is_null() && matches!(req, Request::GetBlockByNumber { .. }) {
                return Err(Error::UnexpectedResponse {
                    message: "block not found (null result)".into(),
                });
            }
            successes.insert(req.cache_key(), result);
        }

        Ok(successes)
    }

    fn sleep_duration(&self, attempt: u32) -> Duration {
        let max_backoff = Duration::from_millis(5_000);
        let multiplier = 2_u32.saturating_pow(attempt);
        self.inner
            .backoff
            .checked_mul(multiplier)
            .map(|d| std::cmp::min(d, max_backoff))
            .unwrap_or(max_backoff)
    }
}

/// Build a JSON-RPC batch payload from a deduplicated request list.
fn build_payload(batch: &[Request]) -> Value {
    let array: Vec<Value> = batch
        .iter()
        .enumerate()
        .map(|(idx, req)| {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": idx,
                "method": req.method(),
                "params": req.params(),
            })
        })
        .collect();
    serde_json::json!(array)
}

/// Load all existing `.json` files from `dir` into the papaya map so that
/// `fetch_or_wait` never touches the filesystem on the hot path.
fn load_disk_cache(dir: impl AsRef<Path>, cache: &PapayaMap<String, Value>) {
    for entry in WalkDir::new(dir.as_ref())
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        if let Ok(data) = fs::read(path)
            && let Ok(value) = serde_json::from_slice::<Value>(&data)
            && let Ok(relative) = path.strip_prefix(dir.as_ref())
        {
            let key = relative.to_string_lossy();
            let key = match key.strip_suffix(".json") {
                Some(k) => k.to_owned(),
                None => key.into_owned(),
            };
            cache.pin().insert(key, value);
        }
    }
}

/// Persist a single entry atomically (temp file + rename).
fn write_disk_cache(base_dir: impl AsRef<Path>, key: &str, value: &Value) -> Result<()> {
    let path = base_dir
        .as_ref()
        .join(PathBuf::from(key))
        .with_extension("json");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec(value)?;
    let temp = path.with_extension("tmp");
    fs::write(&temp, data)?;
    fs::rename(&temp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    use alloy_primitives::Address;
    use anyhow::Result;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;
    use crate::evm::forkdb::{Config, MockTransport, Request, Response, Transport, url_hash};

    /// Regression: a single cached entry must be returned without any RPC
    /// call and must be written to disk exactly once (by the fetcher).
    #[test]
    fn backend_caches_and_dedups_exactly_once() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );

        let tmp = tempdir().unwrap();
        let config = Config::new(url)
            .cache_dir(tmp.path())
            .batch_timeout_ms(0)
            .batch_size(1);
        let backend = SharedBackend::new_with_transport(config, transport.clone());

        let reqs = &[Request::GetChainId { url_hash: url_h }];
        let res = backend.fetch_or_wait(reqs).unwrap();
        assert_eq!(res.len(), 1);

        // Disk must contain the cached entry.
        let expected = tmp
            .path()
            .join("eth_chainId")
            .join(format!("{:x}.json", url_h));
        assert!(
            expected.exists(),
            "disk cache must be written by the fetcher"
        );

        // Second call must hit the global cache and issue zero RPC calls.
        let res2 = backend.fetch_or_wait(reqs).unwrap();
        assert_eq!(res2.len(), 1);
        assert_eq!(
            transport.call_count(url, &payload),
            1,
            "second call must not trigger an RPC"
        );
    }

    /// Two threads requesting the same key concurrently must result in a single
    /// RPC call.
    #[test]
    fn backend_dedups_concurrent_requests() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );
        transport.set_delay(Duration::from_millis(50));

        let config = Config::new(url).batch_timeout_ms(0).batch_size(2);
        let backend = Arc::new(SharedBackend::new_with_transport(config, transport.clone()));

        let barrier = Arc::new(Barrier::new(2));
        let backend2 = Arc::clone(&backend);
        let barrier2 = Arc::clone(&barrier);

        let handle1 = std::thread::spawn(move || {
            barrier.wait();
            backend.fetch_or_wait(&[Request::GetChainId { url_hash: url_h }])
        });
        let handle2 = std::thread::spawn(move || {
            barrier2.wait();
            backend2.fetch_or_wait(&[Request::GetChainId { url_hash: url_h }])
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

    /// Regression: when an in-flight request fails after all retries are
    /// exhausted, waiters must receive the error.  They must NOT silently
    /// retry forever.
    #[test]
    fn backend_waiter_receives_error_on_failure() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        // No mock response registered -> transport error.
        transport.set_delay(Duration::from_millis(20));

        let config = Config::new(url)
            .batch_timeout_ms(0)
            .batch_size(2)
            .retries(0);
        let backend = Arc::new(SharedBackend::new_with_transport(config, transport));

        let barrier = Arc::new(Barrier::new(2));
        let backend2 = Arc::clone(&backend);
        let barrier2 = Arc::clone(&barrier);

        let handle1 = std::thread::spawn(move || {
            barrier.wait();
            backend.fetch_or_wait(&[Request::GetChainId { url_hash: url_h }])
        });
        let handle2 = std::thread::spawn(move || {
            barrier2.wait();
            backend2.fetch_or_wait(&[Request::GetChainId { url_hash: url_h }])
        });

        let res1 = handle1.join().unwrap();
        let res2 = handle2.join().unwrap();

        assert!(
            res1.is_err() || res2.is_err(),
            "at least one waiter must receive an error"
        );
    }

    /// A JSON-RPC batch containing an error object must fail the entire batch.
    #[test]
    fn backend_batch_fails_on_rpc_error() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":["0x0000000000000000000000000000000000000000","0x1"]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x1"},
                {"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"bad block"}}
            ]),
        );

        let config = Config::new(url)
            .batch_size(2)
            .batch_timeout_ms(100)
            .retries(0);
        let backend = SharedBackend::new_with_transport(config, transport);

        let reqs = &[
            Request::GetChainId { url_hash: url_h },
            Request::GetBalance {
                chain_id: 1,
                address: Address::ZERO,
                block: 1,
            },
        ];
        let res = backend.fetch_or_wait(reqs);
        assert!(
            res.is_err(),
            "batch containing an RPC error must fail entirely"
        );
    }

    /// A JSON-RPC batch with a missing response item must fail the entire batch.
    #[test]
    fn backend_batch_fails_on_missing_items() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":["0x0000000000000000000000000000000000000000","0x1"]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );

        let config = Config::new(url)
            .batch_size(2)
            .batch_timeout_ms(100)
            .retries(0);
        let backend = SharedBackend::new_with_transport(config, transport);

        let reqs = &[
            Request::GetChainId { url_hash: url_h },
            Request::GetBalance {
                chain_id: 1,
                address: Address::ZERO,
                block: 1,
            },
        ];
        let res = backend.fetch_or_wait(reqs);
        assert!(
            res.is_err(),
            "batch with a missing response item must fail entirely"
        );
    }

    /// A transient transport error must trigger a whole-batch retry.
    #[test]
    fn backend_retries_on_transient_transport_error() {
        #[derive(Debug)]
        struct FailThenSucceed {
            fail_count: AtomicUsize,
            max_fail: usize,
            response: serde_json::Value,
        }

        impl Transport for FailThenSucceed {
            fn exec(&self, _url: &str, _payload: &serde_json::Value) -> Result<serde_json::Value> {
                let count = self.fail_count.fetch_add(1, Ordering::SeqCst);
                if count < self.max_fail {
                    Err(anyhow::anyhow!("503 Service Unavailable"))
                } else {
                    Ok(self.response.clone())
                }
            }
        }

        let url = "mock://test";
        let url_h = url_hash(url);

        let _payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        let transport = FailThenSucceed {
            fail_count: AtomicUsize::new(0),
            max_fail: 2,
            response: json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        };

        let config = Config::new(url)
            .batch_timeout_ms(0)
            .batch_size(1)
            .retries(3)
            .backoff_ms(0);
        let backend = SharedBackend::new_with_transport(config, transport);

        let res = backend
            .fetch_or_wait(&[Request::GetChainId { url_hash: url_h }])
            .unwrap();
        assert_eq!(res.len(), 1);
        assert!(matches!(&res[0], Response::ChainId(1)));
    }

    /// Regression: two identical requests in the same `fetch_or_wait` slice
    /// must result in exactly one RPC item.
    #[test]
    fn backend_self_dedup_within_slice() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );

        let config = Config::new(url).batch_timeout_ms(0).batch_size(1);
        let backend = SharedBackend::new_with_transport(config, transport.clone());

        let reqs = &[
            Request::GetChainId { url_hash: url_h },
            Request::GetChainId { url_hash: url_h },
        ];
        let res = backend.fetch_or_wait(reqs).unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(
            transport.call_count(url, &payload),
            1,
            "duplicate requests in the same slice must dedup to one RPC item"
        );
    }

    /// Regression: disk cache must use compact JSON, not pretty-printed JSON.
    #[test]
    fn disk_cache_uses_compact_json() {
        let tmp = tempdir().unwrap();
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );

        let config = Config::new(url)
            .cache_dir(tmp.path())
            .batch_timeout_ms(0)
            .batch_size(1);
        let backend = SharedBackend::new_with_transport(config, transport);
        backend
            .fetch_or_wait(&[Request::GetChainId { url_hash: url_h }])
            .unwrap();

        let expected = tmp
            .path()
            .join("eth_chainId")
            .join(format!("{:x}.json", url_h));
        let on_disk = fs::read(&expected).unwrap();
        let value = json!("0x1");
        let compact = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            on_disk, compact,
            "disk cache must use compact JSON instead of pretty-printed JSON"
        );
    }

    /// Regression: a new `SharedBackend` instance must load existing disk cache
    /// into the global map at construction time.
    #[test]
    fn disk_cache_loads_at_startup() {
        let tmp = tempdir().unwrap();
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );

        // First backend writes the cache.
        let config1 = Config::new(url)
            .cache_dir(tmp.path())
            .batch_timeout_ms(0)
            .batch_size(1);
        let backend1 = SharedBackend::new_with_transport(config1, transport.clone());
        backend1
            .fetch_or_wait(&[Request::GetChainId { url_hash: url_h }])
            .unwrap();

        // Second backend loads from disk at startup.
        let config2 = Config::new(url).cache_dir(tmp.path());
        let backend2 = SharedBackend::new_with_transport(config2, transport.clone());
        let res = backend2
            .fetch_or_wait(&[Request::GetChainId { url_hash: url_h }])
            .unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(
            transport.call_count(url, &payload),
            1,
            "second backend must load from disk and not issue a second RPC"
        );
    }

    /// Regression: a single thread calling `fetch_or_wait` must resolve its
    /// own requests when it becomes the fetcher (batch_size = 1).
    #[test]
    fn fetcher_collects_its_own_results() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );

        let config = Config::new(url).batch_timeout_ms(0).batch_size(1);
        let backend = SharedBackend::new_with_transport(config, transport);

        let res = backend
            .fetch_or_wait(&[Request::GetChainId { url_hash: url_h }])
            .unwrap();
        assert_eq!(res.len(), 1);
        assert!(matches!(&res[0], Response::ChainId(1)));
    }

    /// Regression: backoff must be capped so a permanently down endpoint
    /// cannot stall a fuzzer thread for unbounded time.
    #[test]
    fn backoff_is_capped() {
        let config = Config::new("mock://test");
        let backend = SharedBackend::new_with_transport(config, MockTransport::default());

        let cap = Duration::from_millis(5_000);
        assert_eq!(backend.sleep_duration(0), Duration::from_millis(100));
        assert_eq!(backend.sleep_duration(1), Duration::from_millis(200));
        assert_eq!(backend.sleep_duration(5), Duration::from_millis(3_200));
        assert_eq!(
            backend.sleep_duration(10),
            cap,
            "backoff must be capped at 5 s to prevent unbounded growth"
        );
    }

    /// Regression: `sleep_duration` must not panic when `attempt >= 32`.
    #[test]
    fn backoff_does_not_overflow() {
        let config = Config::new("mock://test");
        let backend = SharedBackend::new_with_transport(config, MockTransport::default());

        let cap = Duration::from_millis(5_000);
        assert_eq!(backend.sleep_duration(31), cap);
        assert_eq!(backend.sleep_duration(32), cap);
        assert_eq!(backend.sleep_duration(u32::MAX), cap);
    }

    /// Regression: a transport timeout must preserve the endpoint URL in the
    /// error so callers with multiple RPC endpoints can identify which one
    /// failed.
    #[test]
    fn transport_timeout_preserves_url() {
        #[derive(Debug)]
        struct ErrorTransport {
            message: String,
        }

        impl Transport for ErrorTransport {
            fn exec(&self, _url: &str, _payload: &serde_json::Value) -> Result<serde_json::Value> {
                Err(anyhow::anyhow!("{}", self.message))
            }
        }

        let url = "http://rpc.example";
        let config = Config::new(url)
            .batch_timeout_ms(0)
            .batch_size(1)
            .retries(0);
        let backend = SharedBackend::new_with_transport(
            config,
            ErrorTransport {
                message: "request timed out".into(),
            },
        );

        let res = backend.fetch_or_wait(&[Request::GetChainId { url_hash: 0 }]);
        match res {
            Err(Error::RpcTimeout { url: err_url }) => {
                assert_eq!(err_url, url, "RpcTimeout must preserve the endpoint URL");
            }
            other => panic!("expected RpcTimeout with URL={url}, got {:?}", other),
        }
    }

    /// Regression: a transport rate-limit error must preserve the endpoint
    /// URL in the error.
    #[test]
    fn transport_rate_limit_preserves_url() {
        #[derive(Debug)]
        struct ErrorTransport {
            message: String,
        }

        impl Transport for ErrorTransport {
            fn exec(&self, _url: &str, _payload: &serde_json::Value) -> Result<serde_json::Value> {
                Err(anyhow::anyhow!("{}", self.message))
            }
        }

        let url = "http://rpc.example";
        let config = Config::new(url)
            .batch_timeout_ms(0)
            .batch_size(1)
            .retries(0);
        let backend = SharedBackend::new_with_transport(
            config,
            ErrorTransport {
                message: "429 too many requests".into(),
            },
        );

        let res = backend.fetch_or_wait(&[Request::GetChainId { url_hash: 0 }]);
        match res {
            Err(Error::RateLimited { url: err_url }) => {
                assert_eq!(err_url, url, "RateLimited must preserve the endpoint URL");
            }
            other => panic!("expected RateLimited with URL={url}, got {:?}", other),
        }
    }
}
