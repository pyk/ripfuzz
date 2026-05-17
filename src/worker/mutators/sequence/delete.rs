//! Sequence mutator that deletes a random call.

use crate::corpus::Call;
use crate::worker::mutators::{MutationResult, Mutator};

/// Delete a random call from the sequence.
#[derive(Debug, Default)]
pub struct SequenceDeleteMutator;

impl Mutator for SequenceDeleteMutator {
    fn mutate(&mut self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        if calls.is_empty() {
            return MutationResult::Skipped;
        }
        let idx = rng.usize(0..calls.len());
        calls.remove(idx);
        MutationResult::Mutated
    }
}
