//! Corpus mutator that keeps the tail of a sequence.

use std::sync::{Arc, RwLock};

use crate::corpus::{Call, Corpus};
use crate::worker::mutators::{MutationResult, Mutator};

/// Take the tail of a corpus sequence and keep it, discarding the rest.
#[derive(Debug)]
pub struct SequenceTailMutator {
    corpus: Arc<RwLock<Corpus>>,
}

impl SequenceTailMutator {
    pub fn new(corpus: Arc<RwLock<Corpus>>) -> Self {
        Self { corpus }
    }
}

impl Mutator for SequenceTailMutator {
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
        let tail_len = rng.usize(1..=seq.len());
        *calls = seq[seq.len() - tail_len..].to_vec();
        MutationResult::Mutated
    }
}
