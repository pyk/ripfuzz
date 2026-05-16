//! Sequence mutator that inserts a new random call.

use std::borrow::Cow;
use std::num::NonZeroUsize;

use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    state::HasRand,
};
use libafl_bolts::{Named, rands::Rand};

use crate::corpus;

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

impl Named for SequenceInsertMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceInsertMutator")
    }
}

impl<S: HasRand> Mutator<corpus::CallSequenceInput, S> for SequenceInsertMutator {
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut corpus::CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        if self.selectors.is_empty() {
            return Ok(MutationResult::Skipped);
        }
        let idx = if input.calls.is_empty() {
            0
        } else {
            state.rand_mut().below(
                NonZeroUsize::new(input.calls.len() + 1)
                    .ok_or_else(|| libafl::Error::unknown("non-zero"))?,
            )
        };
        let sel_idx = state.rand_mut().below(
            NonZeroUsize::new(self.selectors.len())
                .ok_or_else(|| libafl::Error::unknown("non-zero"))?,
        );

        let mut block_number_delay = 0u64;
        let mut block_timestamp_delay = 0u64;
        if self.max_block_delay > 0 {
            block_number_delay = state.rand_mut().next() % (self.max_block_delay + 1);
        }
        if self.max_time_delay > 0 {
            block_timestamp_delay = state.rand_mut().next() % (self.max_time_delay + 1);
        }

        let mut call = corpus::Call {
            selector: self.selectors[sel_idx],
            args: vec![0u8; 32 * 3], // up to 3 args of padding
            block_number_delay,
            block_timestamp_delay,
            ..Default::default()
        };
        call.cap_delays();
        input.calls.insert(idx, call);
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
