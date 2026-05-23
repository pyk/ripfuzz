//! Request deduplication table for in-flight RPC calls.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Result, anyhow};
use crossbeam::channel::Sender;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
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
                .complete(&self.key, Err(anyhow!("batcher restarted")));
        }
    }
}

#[derive(Debug)]
pub struct DedupTable {
    /// In-flight keys mapped to the list of channel senders for waiters.
    /// The first caller that inserts the key becomes the leader and must
    /// later call `complete`.  Subsequent callers push a new `Sender` into
    /// the vector, drop the shard lock, and block on the paired `Receiver`.
    ///
    /// Using `DashMap` instead of `Mutex<HashMap>` eliminates the global
    /// lock on the read-heavy `register` path when many fuzzer threads
    /// issue distinct RPC requests concurrently.
    inflight: DashMap<String, Vec<Sender<Arc<DedupPayload>>>>,
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
            inflight: DashMap::new(),
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
        trace!(key = %key, table_size = self.inflight.len(), "dedup register");
        match self.inflight.entry(key.to_owned()) {
            Entry::Occupied(mut occupied) => {
                let (tx, rx) = crossbeam::channel::bounded(1);
                occupied.get_mut().push(tx);
                drop(occupied);
                trace!(key = %key, "dedup hit - waiting on in-flight request");
                let payload = match rx.recv() {
                    Ok(v) => v,
                    Err(_) => return Some(Err(anyhow!("dedup channel closed"))),
                };
                Some(match payload.as_ref() {
                    Ok(v) => Ok(v.clone()),
                    Err(s) => Err(anyhow!(s.clone())),
                })
            }
            Entry::Vacant(vacant) => {
                vacant.insert(Vec::new());
                None
            }
        }
    }

    /// Complete an in-flight request and wake all waiters.
    pub fn complete(&self, key: &str, result: Result<serde_json::Value>) {
        self.complete_count.fetch_add(1, Ordering::SeqCst);
        if let Some((_, senders)) = self.inflight.remove(key) {
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

    /// Number of waiters currently registered for `key`.
    fn waiter_count(table: &DedupTable, key: &str) -> usize {
        table.inflight.get(key).map(|r| r.len()).unwrap_or(0)
    }

    #[test]
    fn dedup_first_caller_wins() {
        let table = Arc::new(DedupTable::new());
        let key = "eth_chainId";

        assert!(table.register(key).is_none());

        let table2 = Arc::clone(&table);
        let waiter = std::thread::spawn(move || table2.register(key));

        std::thread::sleep(std::time::Duration::from_millis(100));
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

        // Poll until every waiter has pushed its sender into the in-flight
        // entry.  A short sleep is still needed because the scheduler may
        // not have started all threads yet, but the polling loop makes the
        // test deterministic rather than relying on a fixed delay.
        let start = std::time::Instant::now();
        while waiter_count(&table, key) < 20 {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "timeout waiting for all waiters to register"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        table.complete(key, Ok("0x1a2b".into()));

        for waiter in waiters {
            let result = waiter.join().unwrap().unwrap().unwrap();
            assert_eq!(result, "0x1a2b");
        }
    }
}
