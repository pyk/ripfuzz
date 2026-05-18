//! Corpus mutator that interleaves two sequences.

use std::sync::{Arc, RwLock};

use crate::corpus::{Call, Corpus};
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Interleave two corpus sequences.
#[derive(Debug)]
pub struct SequenceInterleaveMutator {
    corpus: Arc<RwLock<Corpus>>,
}

impl SequenceInterleaveMutator {
    pub fn new(corpus: Arc<RwLock<Corpus>>) -> Self {
        Self { corpus }
    }
}

impl Mutator for SequenceInterleaveMutator {
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

        let take1 = rng.usize(0..=seq1.len());
        let take2 = rng.usize(0..=seq2.len());

        let slice1 = &seq1[..take1];
        let slice2 = &seq2[..take2];

        let mut iter1 = slice1.iter().cloned();
        let mut iter2 = slice2.iter().cloned();
        let mut new_calls = Vec::with_capacity(slice1.len() + slice2.len());
        loop {
            match (iter1.next(), iter2.next()) {
                (Some(a), Some(b)) => {
                    new_calls.push(a);
                    new_calls.push(b);
                }
                (Some(a), None) => new_calls.push(a),
                (None, Some(b)) => new_calls.push(b),
                (None, None) => break,
            }
        }
        *calls = new_calls;
        MutationResult::Mutated
    }
}
