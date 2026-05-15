use std::borrow::Cow;
use std::num::NonZeroUsize;

use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    state::HasRand,
};
use libafl_bolts::{Named, rands::Rand};

use crate::fuzzer::sequence::CallSequenceInput;

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

impl Named for SequenceDelayMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceDelayMutator")
    }
}

impl<S: HasRand> Mutator<CallSequenceInput, S> for SequenceDelayMutator {
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        if input.calls.is_empty() {
            return Ok(MutationResult::Skipped);
        }
        let idx = state
            .rand_mut()
            .below(NonZeroUsize::new(input.calls.len()).unwrap());
        let call = &mut input.calls[idx];

        if self.max_block_delay > 0 {
            call.block_number_delay = state.rand_mut().next() % (self.max_block_delay + 1);
        }
        if self.max_time_delay > 0 {
            call.block_timestamp_delay = state.rand_mut().next() % (self.max_time_delay + 1);
        }
        call.cap_delays();
        Ok(MutationResult::Mutated)
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _new_corpus_id: Option<CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}
