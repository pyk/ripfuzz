//! Shared corpus state for maxxing fuzzing.

use std::sync::Arc;

use alloy_primitives::U256;
use anyhow::Result;
use parking_lot::RwLock;

use crate::corpus::{Item, SharedCorpus};

/// The best sequence found so far for one max objective.
#[derive(Debug, Clone)]
pub struct MaxBestItem {
    pub value: U256,
    pub item: Item,
}

/// Raw-score extremes tracked for the corpus.
#[derive(Debug, Clone, Copy)]
struct Extremes {
    /// Highest raw `max_*` score observed.
    max: U256,
    /// Lowest raw `max_*` score observed.
    min: U256,
}

/// Inner shared state for the maxxing corpus.
#[derive(Debug)]
struct MaxxingFuzzerCorpusInner {
    /// Coverage-guided corpus.
    corpus: SharedCorpus,
    /// Best derived profit `raw.saturating_sub(baseline)`.
    best: RwLock<Option<MaxBestItem>>,
    /// Baseline raw score after `setup()`.
    baseline: RwLock<Option<U256>>,
    /// Raw-score extremes, initialized to `baseline`.
    extremes: RwLock<Option<Extremes>>,
    /// Protected best id for eviction.
    protected_best: RwLock<Option<String>>,
    /// Protected max extreme id.
    protected_max: RwLock<Option<String>>,
    /// Protected min extreme id.
    protected_min: RwLock<Option<String>>,
}

/// Thread-safe corpus used by maxxing fuzzer threads.
///
/// Wraps the coverage-guided corpus and tracks the best value and sequence for
/// the max objective. Cloning is cheap (shares the same inner state).
#[derive(Debug, Clone)]
pub struct MaxxingFuzzerCorpus {
    inner: Arc<MaxxingFuzzerCorpusInner>,
}

impl MaxxingFuzzerCorpus {
    /// Create a corpus for a single max objective.
    pub fn new(corpus: SharedCorpus) -> Self {
        Self {
            inner: Arc::new(MaxxingFuzzerCorpusInner {
                corpus,
                best: RwLock::new(None),
                baseline: RwLock::new(None),
                extremes: RwLock::new(None),
                protected_best: RwLock::new(None),
                protected_max: RwLock::new(None),
                protected_min: RwLock::new(None),
            }),
        }
    }

    /// Set the baseline score observed after `setup()`.
    ///
    /// The baseline is the raw `max_*` value before any handler. Derived
    /// profit is `score.saturating_sub(baseline)` and extremes are tracked
    /// against the raw score. Calling this more than once is a no-op.
    pub fn set_baseline(&self, baseline: U256) {
        let mut guard = self.inner.baseline.write();
        if guard.is_some() {
            return;
        }
        *guard = Some(baseline);
        drop(guard);
        let mut extremes = self.inner.extremes.write();
        if extremes.is_none() {
            *extremes = Some(Extremes {
                max: baseline,
                min: baseline,
            });
        }
    }

    /// Baseline score, if set.
    pub fn baseline(&self) -> Option<U256> {
        *self.inner.baseline.read()
    }

    /// Return the current best max value, if any.
    pub fn best_value(&self) -> Option<U256> {
        self.inner.best.read().as_ref().map(|best| best.value)
    }

    /// Return the next corpus item for execution.
    pub fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        self.inner.corpus.next_item(rng)
    }

    /// Record that the last picked item did not lead to a new find.
    pub fn note_miss(&self) {
        self.inner.corpus.note_miss();
    }

    /// Add a coverage-interesting sequence to the shared corpus.
    pub fn add_coverage_item(&self, item: Item) -> Result<()> {
        self.inner.corpus.add_item(item)
    }

    /// Record a new best value.
    ///
    /// `raw_score` is the raw `max_*` return. The stored best is the derived
    /// profit `raw_score.saturating_sub(baseline)` where baseline is the value
    /// after `setup()`, falling back to zero when no baseline was set. Returns
    /// `true` when the derived value improved the stored best. The improving
    /// prefix is added to the shared corpus so later campaigns mutate from it.
    pub fn record_improvement(&self, raw_score: U256, item: Item) -> Result<bool> {
        let baseline = self.inner.baseline.read().unwrap_or(U256::ZERO);
        let value = raw_score.saturating_sub(baseline);
        let improved = {
            let mut best = self.inner.best.write();
            let improved = match best.as_ref() {
                Some(current) => value > current.value,
                None => value > U256::ZERO,
            };
            if improved {
                *best = Some(MaxBestItem {
                    value,
                    item: item.clone(),
                });
            }
            improved
        };

        if improved {
            let new_id = item.id();
            self.inner.corpus.add_item(item)?;
            {
                let mut protected = self.inner.protected_best.write();
                let old = protected.clone();
                if old.as_ref() != Some(&new_id) {
                    self.inner.corpus.protect(&new_id);
                    *protected = Some(new_id.clone());
                    if let Some(old_id) = old {
                        self.inner.corpus.unprotect(&old_id);
                    }
                }
            }
        }

        Ok(improved)
    }

    /// Keep a prefix that set a new raw-score extreme.
    ///
    /// The raw `max_*` score is compared against the current max and min
    /// (both initialized to the baseline). When the score is a new max or a
    /// new min the prefix is added to the shared corpus so it can be
    /// mutated. Returns `true` when the prefix was kept.
    pub fn record_extreme(&self, raw_score: U256, item: Item) -> Result<bool> {
        let mut extremes = self.inner.extremes.write();
        let (is_new, is_max) = match extremes.as_mut() {
            Some(extremes) => {
                if raw_score > extremes.max {
                    extremes.max = raw_score;
                    (true, true)
                } else if raw_score < extremes.min {
                    extremes.min = raw_score;
                    (true, false)
                } else {
                    (false, false)
                }
            }
            None => {
                *extremes = Some(Extremes {
                    max: raw_score,
                    min: raw_score,
                });
                (false, false)
            }
        };
        drop(extremes);
        if is_new {
            let new_id = item.id();
            self.inner.corpus.add_item(item)?;
            if is_max {
                let mut protected = self.inner.protected_max.write();
                let old = protected.clone();
                if old.as_ref() != Some(&new_id) {
                    self.inner.corpus.protect(&new_id);
                    *protected = Some(new_id.clone());
                    if let Some(old_id) = old {
                        self.inner.corpus.unprotect(&old_id);
                    }
                }
            } else {
                let mut protected = self.inner.protected_min.write();
                let old = protected.clone();
                if old.as_ref() != Some(&new_id) {
                    self.inner.corpus.protect(&new_id);
                    *protected = Some(new_id.clone());
                    if let Some(old_id) = old {
                        self.inner.corpus.unprotect(&old_id);
                    }
                }
            }
        }
        Ok(is_new)
    }

    /// Snapshot the best item for the max objective.
    pub fn best_item(&self) -> Option<MaxBestItem> {
        self.inner.best.read().clone()
    }

    /// Access the underlying coverage corpus.
    pub fn corpus(&self) -> &SharedCorpus {
        &self.inner.corpus
    }
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;
    use alloy_primitives::U256;

    use crate::corpus::{Call, CorpusConfig, Item, SharedCorpus};

    use super::*;

    fn empty_call() -> Call {
        Call {
            function: Function::parse("foo()").unwrap(),
            args: DynSolValue::Tuple(vec![]),
            ..Default::default()
        }
    }

    #[test]
    fn record_improvement_tracks_and_persists_best() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus = SharedCorpus::new(CorpusConfig::new(tmp.path().join("corpus")));
        let fuzzer_corpus = MaxxingFuzzerCorpus::new(corpus);
        let item = Item::from(vec![empty_call()]);

        assert!(
            !fuzzer_corpus
                .record_improvement(U256::ZERO, item.clone())
                .unwrap()
        );
        assert!(
            fuzzer_corpus
                .record_improvement(U256::from(5), item.clone())
                .unwrap()
        );
        assert!(
            !fuzzer_corpus
                .record_improvement(U256::from(3), item)
                .unwrap()
        );

        let best = fuzzer_corpus.best_item().expect("best must be recorded");
        assert_eq!(best.value, U256::from(5));
        assert_eq!(best.item.calls.len(), 1);
    }

    #[test]
    fn record_improvement_uses_baseline_for_derived_value() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus = SharedCorpus::new(CorpusConfig::new(tmp.path().join("corpus")));
        let fuzzer_corpus = MaxxingFuzzerCorpus::new(corpus);
        fuzzer_corpus.set_baseline(U256::from(100));
        let item = Item::from(vec![empty_call()]);

        assert!(
            !fuzzer_corpus
                .record_improvement(U256::from(90), item.clone())
                .unwrap(),
            "raw below baseline must not improve (value 0)"
        );
        assert!(
            !fuzzer_corpus
                .record_improvement(U256::from(100), item.clone())
                .unwrap(),
            "raw equal to baseline must not improve"
        );
        assert!(
            fuzzer_corpus
                .record_improvement(U256::from(150), item.clone())
                .unwrap(),
            "raw above baseline must improve"
        );
        let best = fuzzer_corpus.best_item().expect("best must be recorded");
        assert_eq!(best.value, U256::from(50));
        assert!(
            !fuzzer_corpus
                .record_improvement(U256::from(120), item)
                .unwrap(),
            "lower derived value must not beat best"
        );
    }

    #[test]
    fn record_extreme_keeps_new_max_and_min() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus = SharedCorpus::new(CorpusConfig::new(tmp.path().join("corpus")));
        let fuzzer_corpus = MaxxingFuzzerCorpus::new(corpus);
        fuzzer_corpus.set_baseline(U256::from(100));
        let item_min = Item::from(vec![empty_call()]);
        let item_max = Item::from(vec![empty_call(), empty_call()]);

        let before = fuzzer_corpus.corpus().stats().item_count;
        assert!(
            fuzzer_corpus
                .record_extreme(U256::from(90), item_min.clone())
                .unwrap(),
            "new min must be kept"
        );
        assert_eq!(fuzzer_corpus.corpus().stats().item_count, before + 1);
        assert!(
            fuzzer_corpus
                .record_extreme(U256::from(110), item_max.clone())
                .unwrap(),
            "new max must be kept"
        );
        assert_eq!(fuzzer_corpus.corpus().stats().item_count, before + 2);
        assert!(
            !fuzzer_corpus
                .record_extreme(U256::from(95), item_min.clone())
                .unwrap(),
            "value between min and max must not be kept"
        );
        assert!(
            !fuzzer_corpus
                .record_extreme(U256::from(105), item_min)
                .unwrap(),
            "value between min and max must not be kept"
        );
    }

    #[test]
    fn old_best_becomes_evictable_after_new_best() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus = SharedCorpus::new(CorpusConfig::new(tmp.path().join("corpus")));
        let fuzzer_corpus = MaxxingFuzzerCorpus::new(corpus);
        fuzzer_corpus.set_baseline(U256::from(100));
        let item1 = Item::from(vec![empty_call()]);
        let item2 = Item::from(vec![empty_call(), empty_call()]);
        assert!(
            fuzzer_corpus
                .record_improvement(U256::from(150), item1.clone())
                .unwrap()
        );
        let id1 = item1.id();
        assert_eq!(
            fuzzer_corpus.inner.protected_best.read().as_ref(),
            Some(&id1)
        );
        assert!(
            fuzzer_corpus
                .record_improvement(U256::from(200), item2.clone())
                .unwrap()
        );
        let id2 = item2.id();
        assert_eq!(
            fuzzer_corpus.inner.protected_best.read().as_ref(),
            Some(&id2)
        );
        assert_ne!(id1, id2);
    }

    #[test]
    fn old_extreme_becomes_evictable_after_new_extreme() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus = SharedCorpus::new(CorpusConfig::new(tmp.path().join("corpus")));
        let fuzzer_corpus = MaxxingFuzzerCorpus::new(corpus);
        fuzzer_corpus.set_baseline(U256::from(100));
        let item_min1 = Item::from(vec![empty_call()]);
        let item_min2 = Item::from(vec![empty_call(), empty_call()]);
        assert!(
            fuzzer_corpus
                .record_extreme(U256::from(90), item_min1.clone())
                .unwrap()
        );
        let id_min1 = item_min1.id();
        assert_eq!(
            fuzzer_corpus.inner.protected_min.read().as_ref(),
            Some(&id_min1)
        );
        assert!(
            fuzzer_corpus
                .record_extreme(U256::from(80), item_min2.clone())
                .unwrap()
        );
        let id_min2 = item_min2.id();
        assert_eq!(
            fuzzer_corpus.inner.protected_min.read().as_ref(),
            Some(&id_min2)
        );
        assert_ne!(id_min1, id_min2);
    }
}
