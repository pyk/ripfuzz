//! Sequence mutator that inserts a new random call.

use crate::fuzzer::corpus::Call;
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Insert a new random call at a random position.
#[derive(Debug, Default)]
pub struct SequenceInsertMutator {
    selectors: Vec<[u8; 4]>,
    max_block_delay: u64,
    max_time_delay: u64,
}

impl SequenceInsertMutator {
    pub fn new(selectors: Vec<[u8; 4]>, max_block_delay: u64, max_time_delay: u64) -> Self {
        Self {
            selectors,
            max_block_delay,
            max_time_delay,
        }
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

        let mut block_number_delay = 0u64;
        let mut block_timestamp_delay = 0u64;
        if self.max_block_delay > 0 {
            block_number_delay = rng.u64(0..self.max_block_delay + 1);
        }
        if self.max_time_delay > 0 {
            block_timestamp_delay = rng.u64(0..self.max_time_delay + 1);
        }

        let mut call = Call {
            selector: self.selectors[sel_idx],
            args: vec![0u8; 32 * 3], // up to 3 args of padding
            block_number_delay,
            block_timestamp_delay,
            ..Default::default()
        };
        call.cap_delays();
        calls.insert(idx, call);
        MutationResult::Mutated
    }
}
