use std::borrow::Cow;
use std::num::NonZeroUsize;

use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    state::HasRand,
};
use libafl_bolts::{Named, rands::Rand};

use crate::fuzzer::sequence::CallSequenceInput;

/// Delete a random call from the sequence.
#[derive(Debug, Default)]
pub struct SequenceDeleteMutator;

impl Named for SequenceDeleteMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceDeleteMutator")
    }
}

impl<S: HasRand> Mutator<CallSequenceInput, S> for SequenceDeleteMutator {
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
