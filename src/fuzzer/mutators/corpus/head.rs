//! Corpus mutator that keeps the head of a sequence.

use std::sync::{Arc, RwLock};

use crate::corpus::{Call, Corpus};
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Take the head of a corpus sequence and keep it, discarding the rest.
#[derive(Debug)]
pub struct SequenceHeadMutator {
    corpus: Arc<RwLock<Corpus>>,
}

impl SequenceHeadMutator {
    pub fn new(corpus: Arc<RwLock<Corpus>>) -> Self {
        Self { corpus }
    }
}

impl Mutator for SequenceHeadMutator {
    fn mutate(&mut self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        let Ok(corpus) = self.corpus.read() else {
            return MutationResult::Skipped;
        };
        let count = corpus.items.len();
        if count == 0 {
            return MutationResult::Skipped;
        }
        let id = rng.usize(0..count);
        let seq = corpus.items[id].calls.clone();
        drop(corpus);
        if seq.is_empty() {
            return MutationResult::Skipped;
        }
        let head_len = rng.usize(1..=seq.len());
        *calls = seq[..head_len].to_vec();
        MutationResult::Mutated
    }
}
