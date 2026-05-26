//! Corpus mutator that keeps the head of a sequence.

use std::sync::Weak;

use crate::fuzzer::corpus::{Call, CorpusItem, SharedCorpusInner};
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Take the head of a corpus sequence and keep it, discarding the rest.
#[derive(Debug)]
pub struct SequenceHeadMutator {
    inner: Weak<SharedCorpusInner>,
}

impl SequenceHeadMutator {
    pub fn new(inner: Weak<SharedCorpusInner>) -> Self {
        Self { inner }
    }
}

impl Mutator for SequenceHeadMutator {
    fn mutate(&self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        let Some(inner) = self.inner.upgrade() else {
            return MutationResult::Skipped;
        };
        let map = inner.items.pin();
        let count = map.len();
        if count == 0 {
            return MutationResult::Skipped;
        }
        let id = rng.usize(0..count);
        let values: Vec<CorpusItem> = map.values().cloned().collect();
        drop(map);
        let seq = values[id].calls.clone();
        if seq.is_empty() {
            return MutationResult::Skipped;
        }
        let head_len = rng.usize(1..=seq.len());
        *calls = seq[..head_len].to_vec();
        MutationResult::Mutated
    }
}
