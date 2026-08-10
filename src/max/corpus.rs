//! Shared corpus state for max-mode fuzzing and shrinking.

use std::sync::Arc;

use alloy_dyn_abi::{DynSolValue, Specifier};
use alloy_json_abi::StateMutability;
use alloy_primitives::U256;
use anyhow::{Result, ensure};
use parking_lot::RwLock;

use crate::corpus::{
    Call, CorpusConfig, ExtractedLiterals, Item, RandomDynSolValue, SharedCorpus, random_uint,
};

/// The best sequence found so far for one max objective.
#[derive(Debug, Clone)]
pub struct MaxBestItem {
    pub value: U256,
    pub item: Item,
}

#[derive(Debug)]
struct MaxFuzzerCorpusInner {
    corpus: SharedCorpus,
    best: RwLock<Option<MaxBestItem>>,
}

/// Thread-safe corpus used by max fuzzer threads.
///
/// Wraps the coverage-guided corpus and tracks the best value and sequence for
/// the max objective. Cloning is cheap (shares the same inner state).
#[derive(Debug, Clone)]
pub struct MaxFuzzerCorpus {
    inner: Arc<MaxFuzzerCorpusInner>,
}

impl MaxFuzzerCorpus {
    /// Create a fuzzer corpus for a single max objective.
    pub fn new(corpus: SharedCorpus) -> Self {
        Self {
            inner: Arc::new(MaxFuzzerCorpusInner {
                corpus,
                best: RwLock::new(None),
            }),
        }
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

/// A single max result being shrunk, shared across shrinker threads.
#[derive(Debug, Clone)]
pub struct MaxShrinkerItem {
    pub value: U256,
    pub item: Item,
}

#[derive(Debug)]
struct MaxShrinkerCorpusInner {
    current: RwLock<MaxShrinkerItem>,
    handler_functions: Vec<alloy_json_abi::Function>,
    literals: ExtractedLiterals,
    corpus: SharedCorpus,
}

/// Thread-safe corpus used by max shrinker threads for one objective.
///
/// Cloning is cheap (shares the same inner state).
#[derive(Debug, Clone)]
pub struct MaxShrinkerCorpus {
    inner: Arc<MaxShrinkerCorpusInner>,
}

impl MaxShrinkerCorpus {
    /// Create a shrinker corpus from a best item and a
    /// [`CorpusConfig`](crate::corpus::CorpusConfig).
    pub fn new(item: Item, value: U256, config: CorpusConfig, corpus: SharedCorpus) -> Self {
        Self {
            inner: Arc::new(MaxShrinkerCorpusInner {
                current: RwLock::new(MaxShrinkerItem { value, item }),
                handler_functions: config.handler_functions,
                literals: config.literals,
                corpus,
            }),
        }
    }

    /// Return a cloned snapshot of the current best item.
    pub fn item(&self) -> MaxShrinkerItem {
        self.inner.current.read().clone()
    }

    /// Return a mutated copy of the current item for the shrinker to try.
    pub fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        let mut item = self.inner.current.read().item.clone();
        let _ = self.mutate_item(rng, &mut item);
        item
    }

    /// Accept a candidate that improves the stored value or shrinks the stored
    /// sequence without losing value.
    ///
    /// Accepted items are persisted to the shared corpus so later campaigns
    /// start from the shrunken result.
    pub fn accept(&self, item: Item, value: U256) {
        let mut current = self.inner.current.write();
        let improves = value > current.value;
        let shrinks = value >= current.value && item.calls.len() < current.item.calls.len();
        if improves || shrinks {
            *current = MaxShrinkerItem {
                value,
                item: item.clone(),
            };
            drop(current);
            let _ = self.inner.corpus.add_item(item);
        }
    }

    /// Remove a random call from the item.
    fn remove_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            item.calls.len() > 1,
            "item must contain at least one call to remove"
        );
        let pos = rng.usize(0..item.calls.len());
        item.calls.remove(pos);
        Ok(())
    }

    /// Apply a randomly selected shrink-oriented mutation to the item.
    fn mutate_item(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        let mut ops = [0u8; 4];
        let mut count = 0usize;

        if item.calls.len() > 1 {
            ops[count] = 0;
            count += 1;
            ops[count] = 1;
            count += 1;
        }
        if !item.calls.is_empty() {
            ops[count] = 2;
            count += 1;
            ops[count] = 3;
            count += 1;
        }

        ensure!(count > 0, "no applicable mutations for this item");

        match ops[rng.usize(0..count)] {
            0 => self.remove_call(rng, item),
            1 => self.swap_call(rng, item),
            2 => self.replace_call(rng, item),
            3 => self.update_args(rng, item),
            _ => unreachable!(),
        }
    }

    fn swap_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        let len = item.calls.len();
        ensure!(len > 1, "item must contain at least two calls to swap");
        let a = rng.usize(0..len);
        let b = (a + rng.usize(1..len)) % len;
        item.calls.swap(a, b);
        Ok(())
    }

    fn replace_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            !item.calls.is_empty(),
            "item must contain at least one call"
        );
        let pos = rng.usize(0..item.calls.len());
        item.calls[pos] = self.generate_call(rng);
        Ok(())
    }

    fn update_args(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            !item.calls.is_empty(),
            "item must contain at least one call"
        );
        let pos = rng.usize(0..item.calls.len());
        let call = &mut item.calls[pos];
        let values: Vec<DynSolValue> = call
            .function
            .inputs
            .iter()
            .filter_map(|p| p.resolve().ok())
            .map(|ty| ty.random(rng, &self.inner.literals))
            .collect();
        call.args = DynSolValue::Tuple(values);
        Ok(())
    }

    fn generate_call(&self, rng: &mut fastrand::Rng) -> Call {
        let functions = &self.inner.handler_functions;
        if functions.is_empty() {
            return Call::default();
        }
        let idx = rng.usize(0..functions.len());
        let func = &functions[idx];
        let values: Vec<DynSolValue> = func
            .inputs
            .iter()
            .filter_map(|p| p.resolve().ok())
            .map(|ty| ty.random(rng, &self.inner.literals))
            .collect();
        let value = if func.state_mutability == StateMutability::Payable {
            Some(random_uint(rng, 256, &self.inner.literals))
        } else {
            None
        };

        Call {
            function: func.clone(),
            args: DynSolValue::Tuple(values),
            value,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;

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
        let fuzzer_corpus = MaxFuzzerCorpus::new(corpus);
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
    fn shrinker_corpus_requires_value_or_size_improvement() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus = SharedCorpus::new(CorpusConfig::new(tmp.path().join("corpus")));
        let functions = vec![Function::parse("foo()").unwrap()];
        let config = CorpusConfig::new(tmp.path().join("corpus"))
            .handler_functions(functions.clone())
            .max_calls(4);
        let seed_item = Item::from(vec![empty_call(), empty_call()]);
        let shrink_corpus = MaxShrinkerCorpus::new(seed_item, U256::from(5), config, corpus);

        // Smaller with the same value is accepted.
        let smaller = Item::from(vec![empty_call()]);
        shrink_corpus.accept(smaller.clone(), U256::from(5));
        assert_eq!(shrink_corpus.item().item.calls.len(), 1);
        assert_eq!(shrink_corpus.item().value, U256::from(5));

        // Same size with a lower value is rejected.
        shrink_corpus.accept(smaller.clone(), U256::from(3));
        assert_eq!(shrink_corpus.item().item.calls.len(), 1);
        assert_eq!(shrink_corpus.item().value, U256::from(5));
    }
}
