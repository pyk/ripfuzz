//! Corpus mutator that splices two sequences together.

use std::sync::{Arc, RwLock};

use crate::fuzzer::corpus::{Call, Corpus};
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Splice two corpus sequences: take the head from one and tail from another.
#[derive(Debug)]
pub struct SequenceSpliceMutator {
    corpus: Arc<RwLock<Corpus>>,
}

impl SequenceSpliceMutator {
    pub fn new(corpus: Arc<RwLock<Corpus>>) -> Self {
        Self { corpus }
    }
}

impl Mutator for SequenceSpliceMutator {
    fn mutate(&mut self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        let Ok(corpus) = self.corpus.read() else {
            return MutationResult::Skipped;
        };
        let count = corpus.items.len();
        if count < 2 {
            return MutationResult::Skipped;
        }

        let id1 = rng.usize(0..count);
        let id2 = rng.usize(0..count);

        let seq1 = corpus.items[id1].calls.clone();
        let seq2 = corpus.items[id2].calls.clone();
        drop(corpus);

        if seq1.is_empty() || seq2.is_empty() {
            return MutationResult::Skipped;
        }

        let head_len = rng.usize(1..=seq1.len());
        let tail_len = rng.usize(0..=seq2.len());

        let mut new_calls = seq1[..head_len].to_vec();
        new_calls.extend_from_slice(&seq2[seq2.len() - tail_len..]);
        *calls = new_calls;
        MutationResult::Mutated
    }
}
