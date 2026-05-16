//! Sequence mutator that deletes a random call.

use std::borrow::Cow;
use std::num::NonZeroUsize;

use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    state::HasRand,
};
use libafl_bolts::{Named, rands::Rand};

use crate::fuzzer::sequence;

/// Delete a random call from the sequence.
#[derive(Debug, Default)]
pub struct SequenceDeleteMutator;

impl Named for SequenceDeleteMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceDeleteMutator")
    }
}

impl<S: HasRand> Mutator<sequence::CallSequenceInput, S> for SequenceDeleteMutator {
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut sequence::CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        if input.calls.is_empty() {
            return Ok(MutationResult::Skipped);
        }
        let idx = state.rand_mut().below(
            NonZeroUsize::new(input.calls.len())
                .ok_or_else(|| libafl::Error::unknown("non-zero"))?,
        );
        input.calls.remove(idx);
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
