//! Corpus mutator that keeps the tail of a sequence.

use std::sync::Weak;

use crate::fuzzer::corpus::{Call, SharedCorpusInner};
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Take the tail of a corpus sequence and keep it, discarding the rest.
#[derive(Debug)]
pub struct SequenceTailMutator {
    inner: Weak<SharedCorpusInner>,
}

impl SequenceTailMutator {
    pub fn new(inner: Weak<SharedCorpusInner>) -> Self {
        Self { inner }
    }
}

impl Mutator for SequenceTailMutator {
    fn mutate(&self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        let Some(inner) = self.inner.upgrade() else {
            return MutationResult::Skipped;
        };
        let Ok(corpus) = inner.corpus.read() else {
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
