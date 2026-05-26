//! Thread-safe corpus with explicit lifecycle phases.

use std::path::Path;
use std::sync::{Arc, RwLock};

use anyhow::Result;

use crate::corpus::{Call, Corpus, CorpusItem};
use crate::evm::coverage::map::LocalCoverage;

/// Thread-safe corpus shared across parallel fuzzer threads.
///
/// Lifecycle:
/// 1. **Loading** - seed or restore items from disk.
/// 2. **Validation** - replay pending items against the chain and keep
///    only the ones that produce new coverage.
/// 3. **Using** - pick items for mutation, record interesting finds,
///    and add failures.
#[derive(Debug, Clone)]
pub struct SharedCorpus {
    inner: Arc<RwLock<Corpus>>,
}

impl SharedCorpus {
    /// Create an empty corpus.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Corpus::new())),
        }
    }

    /// Create a corpus pre-seeded with items.
    pub fn with_seeds(seeds: Vec<CorpusItem>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Corpus::with_seeds(seeds))),
        }
    }

    /// Phase 1: load corpus items from disk into the pending queue.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let corpus = Corpus::load(dir)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(corpus)),
        })
    }

    /// Phase 2: validate all pending items.
    ///
    /// Each pending item is executed by `execute`. If the execution
    /// produces new coverage the item is promoted to the main corpus.
    /// Crashes are recorded as failures.
    pub fn validate_pending(
        &self,
        mut execute: impl FnMut(&[Call]) -> super::engine::ExecutionOutcome,
    ) {
        let pending = {
            let Ok(mut guard) = self.inner.write() else {
                return;
            };
            std::mem::take(&mut guard.pending)
        };

        for item in pending {
            let outcome = execute(&item.calls);

            if outcome.all_ok {
                let Ok(mut guard) = self.inner.write() else {
                    continue;
                };
                let _ = guard.check_and_update_coverage(&outcome.coverage, &item);
            }

            if outcome.crash.is_some() {
                let Ok(mut guard) = self.inner.write() else {
                    continue;
                };
                guard.add_failure(item);
            }
        }
    }

    /// Phase 3: pop a pending item for replay.
    pub fn pop_pending(&self) -> Option<CorpusItem> {
        self.inner.write().ok()?.pop_pending_item()
    }

    /// Phase 3: pick a weighted random item for mutation.
    pub fn pick_for_mutation(&self, rng: &mut fastrand::Rng) -> Option<(usize, CorpusItem)> {
        self.inner
            .read()
            .ok()?
            .random_item_for_mutation_with_index(rng)
    }

    /// Phase 3: record an interesting item (new coverage).
    pub fn record_interesting(&self, item: CorpusItem, coverage: &LocalCoverage) -> bool {
        let Ok(mut guard) = self.inner.write() else {
            return false;
        };
        guard.check_and_update_coverage(coverage, &item)
    }

    /// Phase 3: record that a mutation produced new coverage.
    pub fn record_new_find(&self, idx: usize) {
        let Ok(mut guard) = self.inner.write() else {
            return;
        };
        if let Some(item) = guard.items.get_mut(idx) {
            item.new_finds_produced += 1;
        }
    }

    /// Phase 3: record a mutation attempt on an existing item.
    pub fn record_mutation(&self, idx: usize) {
        let Ok(mut guard) = self.inner.write() else {
            return;
        };
        if let Some(item) = guard.items.get_mut(idx) {
            item.total_mutations += 1;
        }
    }

    /// Phase 3: add an item for mutation if it is not already present.
    pub fn add_item_for_mutation(&self, item: CorpusItem) -> bool {
        let Ok(mut guard) = self.inner.write() else {
            return false;
        };
        guard.add_item_for_mutation(&item)
    }

    /// Phase 3: record a crash as a failure item.
    pub fn record_failure(&self, item: CorpusItem) {
        let Ok(mut guard) = self.inner.write() else {
            return;
        };
        guard.add_failure(item);
    }

    /// Number of coverage-increasing items.
    pub fn item_count(&self) -> usize {
        self.inner.read().map(|g| g.items.len()).unwrap_or(0)
    }

    /// Number of recorded failures.
    pub fn failure_count(&self) -> usize {
        self.inner.read().map(|g| g.failures.len()).unwrap_or(0)
    }

    /// Whether the corpus has entries for mutation.
    pub fn has_entries(&self) -> bool {
        self.item_count() > 0
    }

    /// Total coverage hits.
    pub fn coverage_hits(&self) -> usize {
        self.inner
            .read()
            .map(|g| g.coverage().hit_count())
            .unwrap_or(0)
    }

    /// Access the inner `Arc<RwLock<Corpus>>` for mutators that need it.
    pub fn to_arc(&self) -> Arc<RwLock<Corpus>> {
        Arc::clone(&self.inner)
    }

    /// Clone the global coverage map.
    pub fn coverage_map(&self) -> Option<crate::evm::coverage::map::CoverageMap> {
        let guard = self.inner.read().ok()?;
        let cov = guard.coverage();
        Some(cov.clone())
    }

    /// Persist items and failures to disk.
    pub fn flush_to_disk(&self, dir: impl AsRef<Path>) -> Result<()> {
        let Ok(guard) = self.inner.read() else {
            return Ok(());
        };
        let mut corpus = Corpus::new();
        corpus.items = guard.items.clone();
        corpus.failures = guard.failures.clone();
        corpus.pending = guard.pending.clone();
        corpus.set_coverage(guard.coverage().clone());
        corpus.set_storage_dir(dir);
        corpus.flush_to_disk()
    }
}

impl Default for SharedCorpus {
    fn default() -> Self {
        Self::new()
    }
}
