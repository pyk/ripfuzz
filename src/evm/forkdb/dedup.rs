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

#[derive(Debug, Default)]
struct DedupState {
    senders: Vec<Sender<Arc<DedupPayload>>>,
    completed: Option<Arc<DedupPayload>>,
}

/// Ensures the entry is removed even if the caller panics.
pub struct DedupGuard<'a> {
    table: &'a DedupTable,
    key: String,
    active: bool,
}

impl<'a> DedupGuard<'a> {
    pub fn deactivate(mut self) {
        self.active = false;
        self.table.remove(&self.key);
    }
}

impl Drop for DedupGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            trace!(key = %self.key, "dedup guard dropping - abandoning in-flight request");
            self.table.abandon(&self.key);
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
    inflight: DashMap<String, DedupState>,
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
                let state = occupied.get_mut();
                if let Some(ref payload) = state.completed {
                    let payload = Arc::clone(payload);
                    drop(occupied);
                    Some(match payload.as_ref() {
                        Ok(v) => Ok(v.clone()),
                        Err(s) => Err(anyhow!(s.clone())),
                    })
                } else {
                    let (tx, rx) = crossbeam::channel::bounded(1);
                    state.senders.push(tx);
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
            }
            Entry::Vacant(vacant) => {
                vacant.insert(DedupState::default());
                None
            }
        }
    }

    /// Complete an in-flight request and wake all waiters.
    pub fn complete(&self, key: &str, result: Result<serde_json::Value>) {
        self.complete_count.fetch_add(1, Ordering::SeqCst);
        let payload = Arc::new(match result {
            Ok(v) => Ok(v),
            Err(e) => Err(format!("{e}")),
        });
        let Some(mut state) = self.inflight.get_mut(key) else {
            return;
        };
        if state.completed.is_some() {
            return;
        }
        state.completed = Some(Arc::clone(&payload));
        let senders = std::mem::take(&mut state.senders);
        drop(state);
        for tx in senders {
            let _ = tx.send(Arc::clone(&payload));
        }
    }

    /// Remove the entry for `key`. Called by the leader when it is done.
    pub fn remove(&self, key: &str) {
        self.inflight.remove(key);
    }

    /// Mark an in-flight request as abandoned (e.g. batcher panic) and wake
    /// all waiters with a transient error, then remove the entry.
    pub fn abandon(&self, key: &str) {
        let Entry::Occupied(mut occupied) = self.inflight.entry(key.to_owned()) else {
            return;
        };
        let state = occupied.get_mut();
        if state.completed.is_none() {
            let error_payload = Arc::new(Err("batcher restarted".into()));
            state.completed = Some(Arc::clone(&error_payload));
            let senders = std::mem::take(&mut state.senders);
            drop(occupied);
            for tx in senders {
                let _ = tx.send(Arc::clone(&error_payload));
            }
            self.inflight.remove(key);
        } else {
            occupied.remove();
        }
    }

    /// Create a guard that auto-abandons on panic.
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
        table
            .inflight
            .get(key)
            .map(|r| r.senders.len())
            .unwrap_or(0)
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

    /// Regression: the entry must remain in the table after `complete` so that
    /// late waiters receive the result immediately. It must only be removed when
    /// the leader's guard is explicitly deactivated.
    #[test]
    fn dedup_late_waiter_after_complete() {
        let table = Arc::new(DedupTable::new());
        let key = "eth_chainId";

        // Leader registers.
        assert!(table.register(key).is_none());

        // Complete the request while the leader still holds the guard.
        table.complete(key, Ok("0x1a2b".into()));

        // Before the guard is deactivated, a late waiter must receive the
        // completed result, NOT become a new leader.
        let late_result = table.register(key);
        assert!(
            late_result.is_some(),
            "late waiter must see completed result, not become a new leader"
        );
        assert_eq!(late_result.unwrap().unwrap(), "0x1a2b");

        // After deactivation, the entry is removed and a new caller becomes leader.
        let guard = table.guard(key);
        guard.deactivate();

        assert!(
            table.register(key).is_none(),
            "entry must be removed after guard deactivation"
        );
    }
}
