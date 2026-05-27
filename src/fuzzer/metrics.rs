//! Shared metrics across parallel fuzzer threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Metrics snapshot produced by [`Shared::try_snapshot`].
#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    pub elapsed: Duration,
    pub runs: u64,
    pub calls: u64,
    pub gas: u64,
    pub failures: u64,
}

/// Mutable state held by [`Shared`] behind an [`Arc`].
///
/// All fields are atomics or immutable so clones of [`Shared`] share
/// the same counters without requiring additional synchronization.
#[derive(Debug)]
struct SharedInner {
    runs: AtomicU64,
    calls: AtomicU64,
    gas: AtomicU64,
    failures: AtomicU64,
    last_print: AtomicU64,
    start: Instant,
}

/// Thread-safe metrics shared across all fuzzer threads.
///
/// Only one thread may print per 3-second interval.
#[derive(Debug, Clone)]
pub struct Shared {
    inner: Arc<SharedInner>,
}

impl Shared {
    /// Create fresh metrics.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SharedInner {
                runs: AtomicU64::new(0),
                calls: AtomicU64::new(0),
                gas: AtomicU64::new(0),
                failures: AtomicU64::new(0),
                last_print: AtomicU64::new(0),
                start: Instant::now(),
            }),
        }
    }

    /// Record a completed fuzz iteration.
    pub fn record(&self, calls: u64, gas: u64) {
        self.inner.runs.fetch_add(1, Ordering::Relaxed);
        self.inner.calls.fetch_add(calls, Ordering::Relaxed);
        self.inner.gas.fetch_add(gas, Ordering::Relaxed);
    }

    /// Record a discovered failure.
    pub fn record_failure(&self) {
        self.inner.failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Try to acquire the right to snapshot metrics.
    ///
    /// Returns `Some(snapshot)` only if at least 3 seconds have elapsed
    /// since the last successful snapshot and this thread wins the CAS.
    pub fn try_snapshot(&self) -> Option<Snapshot> {
        let now = self.inner.start.elapsed().as_secs();
        let last = self.inner.last_print.load(Ordering::Relaxed);

        if now < last.saturating_add(3) {
            return None;
        }

        if self
            .inner
            .last_print
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            Some(self.aggregate())
        } else {
            None
        }
    }

    /// Read the current metrics without claiming the snapshot token.
    pub fn aggregate(&self) -> Snapshot {
        Snapshot {
            elapsed: self.inner.start.elapsed(),
            runs: self.inner.runs.load(Ordering::Relaxed),
            calls: self.inner.calls.load(Ordering::Relaxed),
            gas: self.inner.gas.load(Ordering::Relaxed),
            failures: self.inner.failures.load(Ordering::Relaxed),
        }
    }
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}
