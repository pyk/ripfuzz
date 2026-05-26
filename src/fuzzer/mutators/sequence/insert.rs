//! Sequence mutator that inserts a new random call.

use crate::fuzzer::corpus::Call;
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Insert a new random call at a random position.
#[derive(Debug, Default)]
pub struct SequenceInsertMutator {
    selectors: Vec<[u8; 4]>,
}

impl SequenceInsertMutator {
    pub fn new(selectors: Vec<[u8; 4]>) -> Self {
        Self { selectors }
    }
}

impl Mutator for SequenceInsertMutator {
    fn mutate(&self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        if self.selectors.is_empty() {
            return MutationResult::Skipped;
        }
        let idx = if calls.is_empty() {
            0
        } else {
            rng.usize(0..calls.len() + 1)
        };
        let sel_idx = rng.usize(0..self.selectors.len());

        let call = Call {
            selector: self.selectors[sel_idx],
            args: vec![0u8; 32 * 3], // up to 3 args of padding
            ..Default::default()
        };
        calls.insert(idx, call);
        MutationResult::Mutated
    }
}
