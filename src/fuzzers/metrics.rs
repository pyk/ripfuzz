//! Shared metrics across parallel fuzzer threads.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Atomic counter set for one function.
#[derive(Debug)]
struct AtomicFunctionMetrics {
    calls: AtomicU64,
    gas: AtomicU64,
    reverts: AtomicU64,
}

/// Metrics snapshot produced by [`SharedMetrics::try_snapshot`].
#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    pub elapsed: Duration,
    pub runs: u64,
    pub calls: u64,
    pub gas: u64,
}

/// Per-function metrics snapshot (calls, gas, reverts).
#[derive(Debug, Clone, Copy, Default)]
pub struct FunctionMetricsSnapshot {
    pub calls: u64,
    pub gas: u64,
    pub reverts: u64,
}

/// Mutable state held by [`SharedMetrics`] behind an [`Arc`].
///
/// Simple counters use atomics so threads can increment them lock-free.
/// Per-function metrics are stored in a pre-allocated array because each
/// function requires three counters (calls, gas, reverts) and must be
/// dynamically keyed by signature. A read-only `HashMap` maps signatures
/// to indices.
#[derive(Debug)]
struct SharedMetricsInner {
    runs: AtomicU64,
    calls: AtomicU64,
    gas: AtomicU64,
    last_print: AtomicU64,
    start: Instant,
    function_index: HashMap<String, usize>,
    functions: Vec<AtomicFunctionMetrics>,
}

/// Thread-safe metrics shared across all fuzzer threads.
///
/// Only one thread may print per 3-second interval.
#[derive(Debug, Clone)]
pub struct SharedMetrics {
    inner: Arc<SharedMetricsInner>,
}

impl SharedMetrics {
    /// Create fresh metrics with a pre-allocated counter for each function.
    pub fn new(signatures: Vec<String>) -> Self {
        let mut function_index = HashMap::with_capacity(signatures.len());
        let mut functions = Vec::with_capacity(signatures.len());
        for (i, sig) in signatures.into_iter().enumerate() {
            function_index.insert(sig, i);
            functions.push(AtomicFunctionMetrics {
                calls: AtomicU64::new(0),
                gas: AtomicU64::new(0),
                reverts: AtomicU64::new(0),
            });
        }
        Self {
            inner: Arc::new(SharedMetricsInner {
                runs: AtomicU64::new(0),
                calls: AtomicU64::new(0),
                gas: AtomicU64::new(0),
                last_print: AtomicU64::new(0),
                start: Instant::now(),
                function_index,
                functions,
            }),
        }
    }

    /// Record a completed fuzz iteration.
    pub fn record(&self, calls: u64, gas: u64) {
        self.inner.runs.fetch_add(1, Ordering::Relaxed);
        self.inner.calls.fetch_add(calls, Ordering::Relaxed);
        self.inner.gas.fetch_add(gas, Ordering::Relaxed);
    }

    /// Record per-function metrics for a single transaction.
    pub fn record_function(&self, signature: &str, calls: u64, gas: u64, reverts: u64) {
        if let Some(&idx) = self.inner.function_index.get(signature) {
            let metrics = &self.inner.functions[idx];
            metrics.calls.fetch_add(calls, Ordering::Relaxed);
            metrics.gas.fetch_add(gas, Ordering::Relaxed);
            metrics.reverts.fetch_add(reverts, Ordering::Relaxed);
        }
    }

    /// Return a clone of the per-function metrics as a vector of tuples.
    pub fn function_metrics(&self) -> Vec<(String, FunctionMetricsSnapshot)> {
        let mut result = Vec::with_capacity(self.inner.function_index.len());
        // checkrs: allow(clone_in_loops)
        let index = self.inner.function_index.clone();
        for (sig, idx) in index {
            let metrics = &self.inner.functions[idx];
            result.push((
                sig,
                FunctionMetricsSnapshot {
                    calls: metrics.calls.load(Ordering::Relaxed),
                    gas: metrics.gas.load(Ordering::Relaxed),
                    reverts: metrics.reverts.load(Ordering::Relaxed),
                },
            ));
        }
        result
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
        }
    }
}

impl Default for SharedMetrics {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}
