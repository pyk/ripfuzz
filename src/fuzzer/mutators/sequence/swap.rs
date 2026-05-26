//! Sequence mutator that swaps two random calls.

use crate::fuzzer::corpus::Call;
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Swap two random calls in the sequence.
#[derive(Debug, Default)]
pub struct SequenceSwapMutator;

impl Mutator for SequenceSwapMutator {
    fn mutate(&self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        if calls.len() < 2 {
            return MutationResult::Skipped;
        }
        let idx1 = rng.usize(0..calls.len());
        let idx2 = rng.usize(0..calls.len());
        if idx1 != idx2 {
            calls.swap(idx1, idx2);
            MutationResult::Mutated
        } else {
            MutationResult::Skipped
        }
    }
}
