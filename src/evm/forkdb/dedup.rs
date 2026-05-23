//! Request deduplication table for in-flight RPC calls.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use crossbeam::channel::Sender;
use tracing::trace;

/// Cloneable payload used inside crossbeam channels. `anyhow::Error` does not
/// implement `Clone`, so we normalise errors to `String` for broadcast.
type DedupPayload = Result<serde_json::Value, String>;

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
    /// In-flight keys mapped to the list of channel senders for waiters.
    /// The first caller that inserts the key becomes the leader and must
    /// later call `complete`.  Subsequent callers push a new `Sender` into
    /// the vector, drop the map lock, and block on the paired `Receiver`.
    inflight: Mutex<HashMap<String, Vec<Sender<Arc<DedupPayload>>>>>,
    pub complete_count: AtomicUsize,
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
            complete_count: AtomicUsize::new(0),
        }
    }

    /// Register a new in-flight request.
    ///
    /// Returns `None` if this thread is the first to issue the request;
    /// the caller must later call `complete` (or use a [`DedupGuard`]).
    /// Returns `Some(result)` if another thread is already handling it,
    /// blocking until the batcher signals completion.
    pub fn register(&self, key: &str) -> Option<Result<serde_json::Value>> {
        let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        trace!(key = %key, table_size = map.len(), "dedup register");
        if let Some(senders) = map.get_mut(key) {
            let (tx, rx) = crossbeam::channel::bounded(1);
            senders.push(tx);
            drop(map);
            trace!(key = %key, "dedup hit - waiting on in-flight request");
            let payload = match rx.recv() {
                Ok(v) => v,
                Err(_) => return Some(Err(anyhow!("dedup channel closed"))),
            };
            return Some(match payload.as_ref() {
                Ok(v) => Ok(v.clone()),
                Err(s) => Err(anyhow!(s.clone())),
            });
        }
        map.insert(key.into(), Vec::new());
        None
    }

    /// Complete an in-flight request and wake all waiters.
    pub fn complete(&self, key: &str, result: Result<serde_json::Value>) {
        self.complete_count.fetch_add(1, Ordering::SeqCst);
        let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(senders) = map.remove(key) {
            let payload = Arc::new(match result {
                Ok(v) => Ok(v),
                Err(e) => Err(format!("{e}")),
            });
            for tx in senders {
                let _ = tx.send(Arc::clone(&payload));
            }
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

    /// Regression: many concurrent waiters on the same key must all receive the
    /// result without spurious wakeup issues.
    #[test]
    fn dedup_many_waiters_all_receive() {
        let table = Arc::new(DedupTable::new());
        let key = "eth_chainId";

        assert!(table.register(key).is_none());

        let waiters: Vec<std::thread::JoinHandle<Option<Result<serde_json::Value>>>> = (0..20)
            .map(|_| {
                let table = Arc::clone(&table);
                std::thread::spawn(move || table.register(key))
            })
            .collect();

        std::thread::sleep(std::time::Duration::from_millis(10));
        table.complete(key, Ok("0x1a2b".into()));

        for waiter in waiters {
            let result = waiter.join().unwrap().unwrap().unwrap();
            assert_eq!(result, "0x1a2b");
        }
    }
}
