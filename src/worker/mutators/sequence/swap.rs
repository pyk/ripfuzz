//! Sequence mutator that swaps two random calls.

use std::borrow::Cow;
use std::num::NonZeroUsize;

use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    state::HasRand,
};
use libafl_bolts::{Named, rands::Rand};

use crate::campaign::input;

/// Swap two random calls in the sequence.
#[derive(Debug, Default)]
pub struct SequenceSwapMutator;

impl Named for SequenceSwapMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceSwapMutator")
    }
}

impl<S: HasRand> Mutator<input::CallSequenceInput, S> for SequenceSwapMutator {
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut input::CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        if input.calls.len() < 2 {
            return Ok(MutationResult::Skipped);
        }
        let idx1 = state.rand_mut().below(
            NonZeroUsize::new(input.calls.len())
                .ok_or_else(|| libafl::Error::unknown("non-zero"))?,
        );
        let idx2 = state.rand_mut().below(
            NonZeroUsize::new(input.calls.len())
                .ok_or_else(|| libafl::Error::unknown("non-zero"))?,
        );
        if idx1 != idx2 {
            input.calls.swap(idx1, idx2);
            Ok(MutationResult::Mutated)
        } else {
            Ok(MutationResult::Skipped)
        }
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _new_corpus_id: Option<CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}
