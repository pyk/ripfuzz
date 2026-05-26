//! Sequence mutator that randomizes block delays.

use crate::fuzzer::corpus::Call;
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Mutate block delays on a random call in the sequence.
#[derive(Debug, Default)]
pub struct SequenceDelayMutator {
    max_block_delay: u64,
    max_time_delay: u64,
}

impl SequenceDelayMutator {
    pub fn new(max_block_delay: u64, max_time_delay: u64) -> Self {
        Self {
            max_block_delay,
            max_time_delay,
        }
    }
}

impl Mutator for SequenceDelayMutator {
    fn mutate(&self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        if calls.is_empty() {
            return MutationResult::Skipped;
        }
        let idx = rng.usize(0..calls.len());
        let call = &mut calls[idx];

        if self.max_block_delay > 0 {
            call.block_number_delay = rng.u64(0..self.max_block_delay + 1);
        }
        if self.max_time_delay > 0 {
            call.block_timestamp_delay = rng.u64(0..self.max_time_delay + 1);
        }
        call.cap_delays();
        MutationResult::Mutated
    }
}
