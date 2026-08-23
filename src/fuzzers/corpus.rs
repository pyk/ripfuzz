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

#[derive(Debug)]
struct MaxxingFuzzerCorpusInner {
    corpus: SharedCorpus,
    best: RwLock<Option<MaxBestItem>>,
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
            }),
        }
    }

    /// Return the current best max value, if any.
    pub fn best_value(&self) -> Option<U256> {
        self.inner.best.read().as_ref().map(|best| best.value)
    }

    /// Return the next corpus item for execution.
    pub fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        self.inner.corpus.next_item(rng)
    }

    /// Add a coverage-interesting sequence to the shared corpus.
    pub fn add_coverage_item(&self, item: Item) -> Result<()> {
        self.inner.corpus.add_item(item)
    }

    /// Record a new best value.
    ///
    /// Returns `true` when the value improved the stored best. The improving
    /// prefix is added to the shared corpus so later campaigns mutate from it.
    pub fn record_improvement(&self, value: U256, item: Item) -> Result<bool> {
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
            self.inner.corpus.add_item(item)?;
        }

        Ok(improved)
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
}
