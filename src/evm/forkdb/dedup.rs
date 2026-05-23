//! Request deduplication table for in-flight RPC calls.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use anyhow::{Result, anyhow};
use tracing::trace;

#[derive(Debug)]
struct InflightHandle {
    done: Condvar,
    completed: AtomicBool,
    result: Mutex<Option<Result<serde_json::Value>>>,
}

impl InflightHandle {
    fn wait(&self) -> Result<serde_json::Value> {
        let mut guard = self.result.lock().unwrap_or_else(|e| e.into_inner());
        while !self.completed.load(Ordering::Relaxed) {
            guard = self.done.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        match guard.as_ref() {
            Some(Ok(v)) => Ok(v.clone()),
            Some(Err(e)) => Err(anyhow!("{e}")),
            None => Err(anyhow!("dedup waiter woken with no result")),
        }
    }
}

/// Ensures `complete` is called even if the caller panics.
pub struct DedupGuard<'a> {
    table: &'a DedupTable,
    key: String,
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
            trace!(key = %self.key, "dedup guard dropping - completing with error");
            self.table
                .complete(&self.key, Err(anyhow!("dedup caller panicked")));
        }
    }
}

#[derive(Debug)]
pub struct DedupTable {
    inflight: Mutex<HashMap<String, Arc<InflightHandle>>>,
}

impl Default for DedupTable {
    fn default() -> Self {
        Self::new()
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
    pub fn register(&self, key: &str) -> Option<Result<serde_json::Value>> {
        let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        trace!(key = %key, table_size = map.len(), "dedup register");
        if let Some(handle) = map.get(key) {
            let handle = Arc::clone(handle);
            drop(map);
            trace!(key = %key, "dedup hit - waiting on in-flight request");
            return Some(handle.wait());
        }
        let handle = Arc::new(InflightHandle {
            done: Condvar::new(),
            completed: AtomicBool::new(false),
            result: Mutex::new(None),
        });
        map.insert(key.into(), handle);
        None
    }

    /// Complete an in-flight request and wake all waiters.
    pub fn complete(&self, key: &str, result: Result<serde_json::Value>) {
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
    pub fn guard(&self, key: &str) -> DedupGuard<'_> {
        DedupGuard {
            table: self,
            key: key.to_owned(),
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
        let key = "eth_chainId";

        assert!(table.register(key).is_none());

        let table2 = Arc::clone(&table);
        let waiter = std::thread::spawn(move || table2.register(key));

        std::thread::sleep(std::time::Duration::from_millis(10));
        table.complete(key, Ok("0x1a2b".into()));

        let result = waiter.join().unwrap().unwrap().unwrap();
        assert_eq!(result, "0x1a2b");
    }

    #[test]
    fn dedup_error_propagation() {
        let table = Arc::new(DedupTable::new());
        let key = "eth_chainId";

        assert!(table.register(key).is_none());

        let table2 = Arc::clone(&table);
        let waiter = std::thread::spawn(move || table2.register(key));

        std::thread::sleep(std::time::Duration::from_millis(10));
        table.complete(key, Err(anyhow!("network down")));

        let err = waiter.join().unwrap().unwrap().unwrap_err();
        assert!(format!("{err}").contains("network down"));
    }
}
