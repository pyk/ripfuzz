//! Shared metrics across parallel fuzzer threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Metrics snapshot produced by [`SharedMetrics::maybe_print`].
#[derive(Debug, Clone, Copy)]
pub struct MetricsSnapshot {
    pub elapsed: Duration,
    pub runs: u64,
    pub calls: u64,
    pub gas: u64,
    pub failures: u64,
}

/// Thread-safe metrics shared across all fuzzer threads.
///
/// Only one thread may print per 3-second interval.
#[derive(Debug, Clone)]
pub struct SharedMetrics {
    runs: Arc<AtomicU64>,
    calls: Arc<AtomicU64>,
    gas: Arc<AtomicU64>,
    failures: Arc<AtomicU64>,
    last_print: Arc<AtomicU64>,
    start: Instant,
}

impl SharedMetrics {
    /// Create fresh metrics.
    pub fn new() -> Self {
        Self {
            runs: Arc::new(AtomicU64::new(0)),
            calls: Arc::new(AtomicU64::new(0)),
            gas: Arc::new(AtomicU64::new(0)),
            failures: Arc::new(AtomicU64::new(0)),
            last_print: Arc::new(AtomicU64::new(0)),
            start: Instant::now(),
        }
    }

    /// Record a completed fuzz iteration.
    pub fn record(&self, calls: u64, gas: u64) {
        self.runs.fetch_add(1, Ordering::Relaxed);
        self.calls.fetch_add(calls, Ordering::Relaxed);
        self.gas.fetch_add(gas, Ordering::Relaxed);
    }

    /// Record a discovered failure.
    pub fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Try to acquire the right to print metrics.
    ///
    /// Returns `Some(snapshot)` only if at least 3 seconds have elapsed
    /// since the last successful print and this thread wins the CAS.
    pub fn maybe_print(&self) -> Option<MetricsSnapshot> {
        let now = self.start.elapsed().as_secs();
        let last = self.last_print.load(Ordering::Relaxed);

        if now < last.saturating_add(3) {
            return None;
        }

        if self
            .last_print
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            Some(self.aggregate())
        } else {
            None
        }
    }

    /// Read the current metrics without claiming the print token.
    pub fn aggregate(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            elapsed: self.start.elapsed(),
            runs: self.runs.load(Ordering::Relaxed),
            calls: self.calls.load(Ordering::Relaxed),
            gas: self.gas.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
        }
    }
}

impl Default for SharedMetrics {
    fn default() -> Self {
        Self::new()
    }
}
