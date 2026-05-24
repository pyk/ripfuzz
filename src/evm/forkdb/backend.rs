//! SharedBackend: lock-free global cache + batched RPC fetcher.
//!
//! ## Design Goal
//!
//! Reduce the number of HTTP round-trips during fuzzing by letting the fuzzer
//! threads themselves batch and deduplicate RPC requests.  There is **no**
//! background worker thread.
//!
//! ## SharedBackend Initialization Phase
//!
//! When a [`SharedBackend`] is created it pre-populates a lock-free
//! `papaya::HashMap` (the *global cache*) from the on-disk cache directory.
//! Every fuzzer thread sees the same map, so a value cached by one thread is
//! instantly visible to all others.
//!
//! ## SharedBackend Execution Phase
//!
//! A fuzzer thread calls [`SharedBackend::fetch_or_wait`] with a slice of
//! [`Request`]s.
//!
//! 1. **Fast path** – every request is already in `global_cache`.  The
//!    function returns the parsed [`Response`]s immediately without locking.
//!
//! 2. **Slow path** – at least one request is missing.  The thread acquires
//!    the global `batch_state` mutex, adds its missing requests to the
//!    pending set (deduplicated by `cache_key`), and either:
//!    * becomes the **fetcher** when `batch_size` is reached or the batch
//!      deadline (default 50 ms) has expired; or
//!    * releases the mutex and blocks on a `Condvar` while waiting for a
//!      fetcher to complete.
//!
//!    The fetcher takes ownership of the pending slice, drops the mutex,
//!    issues a single JSON-RPC batch via `ureq`, retries failed items with
//!    exponential back-off, inserts successful responses into `global_cache`
//!    (and writes them to disk), publishes any errors back into `batch_state`,
//!    and finally wakes all waiting threads with `notify_all`.
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
//! fetcher performs I/O (mutex released) another thread may become the fetcher
//! for the subsequent batch.

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
    /// Errors from the most recent batch that have not yet been claimed.
    errors: HashMap<String, Arc<Error>>,
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
                errors: HashMap::new(),
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

        // Fast path: try to satisfy everything from the global cache without
        // locking.
        {
            let map = self.inner.global_cache.pin();
            let mut results = Vec::with_capacity(reqs.len());
            let mut all_cached = true;
            for req in reqs {
                if let Some(value) = map.get(&req.cache_key()) {
                    results.push(Some(Response::parse(req, value)?));
                } else {
                    all_cached = false;
                    break;
                }
            }
            if all_cached {
                return Ok(results.into_iter().flatten().collect());
            }
        }

        // Slow path: coordinate with other threads.
        let mut state = self.inner.batch_state.lock();

        loop {
            // Re-check cache and any unclaimed errors under the lock.
            let mut all_ready = true;
            let mut results: Vec<Option<Response>> = Vec::with_capacity(reqs.len());
            let mut missing = Vec::new();

            for req in reqs {
                let key = req.cache_key();
                let map = self.inner.global_cache.pin();
                match map.get(&key) {
                    Some(value) => results.push(Some(Response::parse(req, value)?)),
                    None => {
                        results.push(None);
                        all_ready = false;
                        missing.push(key);
                    }
                }
            }

            if all_ready {
                return results
                    .into_iter()
                    .map(|o| {
                        o.ok_or_else(|| Error::UnexpectedResponse {
                            message: "missing result".into(),
                        })
                    })
                    .collect();
            }

            if let Some(key) = missing.iter().find(|k| state.errors.contains_key(*k)) {
                let key: String = key.into();
                let err = Self::take_error(&mut state, &key)?;
                return Err(err);
            }

            // Add missing requests to the current batch, clearing any stale
            // error so the key is retried.
            let to_add: Vec<Request> = reqs
                .iter()
                .filter(|req| {
                    let key = req.cache_key();
                    state.errors.remove(&key);
                    state.keys.insert(key)
                })
                .cloned()
                .collect();
            state.pending.extend(to_add);

            if state.deadline.is_none() && !state.pending.is_empty() {
                state.deadline = Some(Instant::now() + self.inner.batch_timeout);
            }

            let now = Instant::now();
            let deadline_hit = match state.deadline {
                Some(d) => now >= d,
                None => false,
            };
            let size_hit = state.pending.len() >= self.inner.batch_size;

            if (size_hit || deadline_hit) && !state.fetcher_in_flight {
                // Become the fetcher.
                state.fetcher_in_flight = true;
                let batch = std::mem::take(&mut state.pending);
                state.keys.clear();
                state.deadline = None;
                state.errors.clear();
                // Drop the lock while performing I/O so other threads can
                // accumulate the next batch.
                drop(state);

                let (successes, errors) = self.execute_batch(batch)?;

                // Re-acquire lock, publish results, and wake waiters.
                state = self.inner.batch_state.lock();
                state.fetcher_in_flight = false;
                for (key, value) in successes {
                    if let Some(ref dir) = self.inner.cache_dir {
                        let _ = write_disk_cache(dir, &key, &value);
                    }
                    let map = self.inner.global_cache.pin();
                    map.insert(key, value);
                }
                for (key, err) in errors {
                    state.errors.insert(key, err);
                }
                self.inner.batch_condvar.notify_all();
                // Loop again to collect our own results from cache / errors.
            } else {
                // Wait for another thread to finish a batch.
                let timeout = state.deadline.map_or(self.inner.batch_timeout, |d| {
                    d.saturating_duration_since(now)
                });
                self.inner.batch_condvar.wait_for(&mut state, timeout);
                // Spurious wakeup or timeout: loop and re-check cache.
            }
        }
    }

    /// Remove and return a per-key error from `batch_state`.  If the
    /// underlying `Arc` is shared with other keys, the error is cloned.
    fn take_error(state: &mut BatchState, key: &str) -> Result<Error, Error> {
        let err = state
            .errors
            .remove(key)
            .ok_or_else(|| Error::UnexpectedResponse {
                message: "stale error key".into(),
            })?;
        match Arc::try_unwrap(err) {
            Ok(e) => Ok(e),
            Err(e) => Ok((*e).clone()),
        }
    }

    /// Execute a JSON-RPC batch, retrying failed items individually.
    #[allow(clippy::type_complexity)]
    fn execute_batch(
        &self,
        batch: Vec<Request>,
    ) -> Result<(HashMap<String, Value>, HashMap<String, Arc<Error>>), Error> {
        // Deduplicate by cache key.
        let mut deduped: Vec<(String, Request)> = Vec::with_capacity(batch.len());
        let mut seen = HashSet::new();
        for req in batch {
            let key = req.cache_key();
            if seen.insert(key) {
                deduped.push((req.cache_key(), req));
            }
        }

        let mut payload = build_payload(&deduped);

        // Rate limit gate: one HTTP POST == one token regardless of batch size.
        if let Some(ref limiter) = self.inner.limiter {
            limiter.acquire();
        }

        // Live network fetch with exponential backoff retries.
        let mut all_successes = HashMap::new();
        let mut all_errors: HashMap<String, Arc<Error>> = HashMap::new();
        let mut last_err: Option<Error> = None;
        for attempt in 0..=self.inner.retries {
            match self.inner.transport.exec(&self.inner.url, &payload) {
                Ok(v) => {
                    let arr: Vec<Value> = if v.is_object() {
                        vec![v]
                    } else {
                        v.as_array().cloned().unwrap_or_default()
                    };

                    let mut by_id: HashMap<usize, Value> = HashMap::new();
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
                    for (idx, (key, req)) in deduped.into_iter().enumerate() {
                        if let Some(result) = by_id.remove(&idx) {
                            all_successes.insert(key, result);
                        } else {
                            next_batch.push((key, req));
                        }
                    }

                    if next_batch.is_empty() {
                        return Ok((all_successes, all_errors));
                    }

                    // Some items failed or were missing - retry only them.
                    deduped = next_batch;
                    if attempt < self.inner.retries {
                        payload = build_payload(&deduped);
                        std::thread::sleep(self.sleep_duration(attempt));
                        continue;
                    }

                    // Retries exhausted: return errors for the remaining items.
                    let err = last_err.unwrap_or_else(|| Error::UnexpectedResponse {
                        message: "RPC request failed or response missing".into(),
                    });
                    let arc_err = Arc::new(err);
                    for (key, _) in deduped {
                        all_errors.insert(key, Arc::clone(&arc_err));
                    }
                    return Ok((all_successes, all_errors));
                }
                Err(e) => {
                    last_err = Some(Error::from_anyhow(e, &self.inner.url));
                    if attempt < self.inner.retries {
                        std::thread::sleep(self.sleep_duration(attempt));
                    }
                }
            }
        }

        let err = last_err.unwrap_or_else(|| Error::UnexpectedResponse {
            message: "RPC request failed".into(),
        });
        let arc_err = Arc::new(err);
        for (key, _) in deduped {
            all_errors.insert(key, Arc::clone(&arc_err));
        }
        Ok((all_successes, all_errors))
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

fn build_payload(batch: &[(String, Request)]) -> Value {
    let array: Vec<Value> = batch
        .iter()
        .enumerate()
        .map(|(idx, (_, req))| {
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
            let guard = cache.guard();
            cache.insert(key, value, &guard);
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
    use std::sync::Arc;
    use std::sync::Barrier;
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

    /// A JSON-RPC batch containing one error and one success must not fail the
    /// entire batch.  The successful item is cached and only the failed item is
    /// retried.
    #[test]
    fn backend_batch_per_item_retry() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let url_h = url_hash(url);

        let batch_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":["0x0000000000000000000000000000000000000000","0x1"]}
        ]);
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
        let backend = SharedBackend::new_with_transport(config, transport.clone());

        let reqs = &[
            Request::GetChainId { url_hash: url_h },
            Request::GetBalance {
                chain_id: 1,
                address: Address::ZERO,
                block: 1,
            },
        ];
        let res = backend.fetch_or_wait(reqs).unwrap();
        assert_eq!(res.len(), 2);
        assert!(matches!(&res[0], Response::ChainId(1)));
        assert!(matches!(&res[1], Response::Balance(v) if v.is_zero()));

        assert_eq!(
            transport.call_count(url, &batch_payload),
            1,
            "full batch should be sent once"
        );
        assert_eq!(
            transport.call_count(url, &single_payload),
            1,
            "failed item should be retried individually"
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

    /// Regression: non-compliant RPC servers may return a single JSON object
    /// instead of a single-element array for a batch of one request.
    #[test]
    fn single_object_batch_response_is_accepted() {
        let transport = MockTransport::default();
        let url = "mock://test";

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!({
                "jsonrpc": "2.0",
                "id": 0,
                "result": "0x1",
            }),
        );

        let config = Config::new(url).batch_timeout_ms(0).batch_size(1);
        let backend = SharedBackend::new_with_transport(config, transport);

        let res = backend
            .fetch_or_wait(&[Request::GetChainId { url_hash: 0 }])
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

    /// Regression: the fetcher thread must be able to wait on itself when it
    /// becomes a fetcher for a batch that includes its own requests.
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
