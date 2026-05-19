//! In-flight request deduplication.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use anyhow::anyhow;
use tracing::trace;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RequestKey {
    method: String,
    args_json: String,
}

impl RequestKey {
    pub fn new(method: &str, params: &[serde_json::Value]) -> Self {
        Self {
            method: method.into(),
            args_json: serde_json::to_string(params).unwrap_or_default(),
        }
    }
}

impl std::fmt::Display for RequestKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.method, self.args_json)
    }
}

struct InflightHandle {
    done: Condvar,
    completed: AtomicBool,
    result: Mutex<Option<anyhow::Result<serde_json::Value>>>,
}

impl InflightHandle {
    fn wait(&self) -> anyhow::Result<serde_json::Value> {
        let mut guard = self.result.lock().unwrap_or_else(|e| e.into_inner());
        while !self.completed.load(Ordering::Relaxed) {
            guard = self.done.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        guard
            .take()
            .unwrap_or_else(|| Err(anyhow!("dedup waiter woken with no result")))
    }
}

/// Ensures `complete` is called even if the caller panics.
pub struct DedupGuard<'a> {
    table: &'a DedupTable,
    key: &'a RequestKey,
    active: bool,
}

impl<'a> DedupGuard<'a> {
    pub fn deactivate(mut self) {
        self.active = false;
    }
}

impl Drop for DedupGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            trace!(key = %self.key, "dedup guard dropping — completing with error");
            self.table
                .complete(self.key, Err(anyhow!("dedup caller panicked")));
        }
    }
}

pub struct DedupTable {
    inflight: Mutex<HashMap<RequestKey, Arc<InflightHandle>>>,
}

impl std::fmt::Debug for DedupTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("DedupTable")
            .field("inflight_count", &guard.len())
            .finish()
    }
}

impl DedupTable {
    pub fn new() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new in-flight request.
    ///
    /// Returns `None` if this thread is the first to issue the request;
    /// the caller must later call `complete` (or use a [`DedupGuard`]).
    /// Returns `Some(handle)` if another thread is already handling it.
    pub fn register(&self, key: &RequestKey) -> Option<anyhow::Result<serde_json::Value>> {
        let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        trace!(key = %key, table_size = map.len(), "dedup register");
        if let Some(handle) = map.get(key) {
            let handle = Arc::clone(handle);
            drop(map);
            trace!(key = %key, "dedup hit — waiting on in-flight request");
            return Some(handle.wait());
        }
        let handle = Arc::new(InflightHandle {
            done: Condvar::new(),
            completed: AtomicBool::new(false),
            result: Mutex::new(None),
        });
        map.insert(key.clone(), handle);
        None
    }

    /// Complete an in-flight request and wake all waiters.
    pub fn complete(&self, key: &RequestKey, result: anyhow::Result<serde_json::Value>) {
        let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = map.remove(key) {
            let mut guard = handle.result.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(result);
            handle.completed.store(true, Ordering::Relaxed);
            drop(guard);
            handle.done.notify_all();
        }
    }

    /// Create a guard that auto-completes with error on panic.
    pub fn guard<'a>(&'a self, key: &'a RequestKey) -> DedupGuard<'a> {
        DedupGuard {
            table: self,
            key,
            active: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn dedup_first_caller_wins() {
        let table = Arc::new(DedupTable::new());
        let key = RequestKey::new("eth_blockNumber", &[]);

        // First registration returns None (caller must do the work).
        assert!(table.register(&key).is_none());

        // Second registration blocks until the first caller completes.
        let table2 = Arc::clone(&table);
        let key2 = key.clone();
        let waiter = std::thread::spawn(move || table2.register(&key2));

        std::thread::sleep(std::time::Duration::from_millis(10));
        table.complete(&key, Ok("0x1a2b".into()));

        let result = waiter.join().unwrap().unwrap().unwrap();
        assert_eq!(result, "0x1a2b");
    }

    #[test]
    fn dedup_error_propagation() {
        let table = Arc::new(DedupTable::new());
        let key = RequestKey::new("eth_blockNumber", &[]);

        assert!(table.register(&key).is_none());

        let table2 = Arc::clone(&table);
        let key2 = key.clone();
        let waiter = std::thread::spawn(move || table2.register(&key2));

        std::thread::sleep(std::time::Duration::from_millis(10));
        table.complete(&key, Err(anyhow::anyhow!("network down")));

        let err = waiter.join().unwrap().unwrap().unwrap_err();
        assert!(format!("{err}").contains("network down"));
    }
}
