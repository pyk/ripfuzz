//! Corpus mutator that keeps the tail of a sequence.

use std::borrow::Cow;
use std::num::NonZeroUsize;

use libafl::{
    corpus::{Corpus, CorpusId},
    mutators::{MutationResult, Mutator},
    state::{HasCorpus, HasRand},
};
use libafl_bolts::Named;
use libafl_bolts::rands::Rand;

use crate::campaign::input;

/// Take the tail of a corpus sequence and keep it, discarding the rest.
#[derive(Debug, Default)]
pub struct SequenceTailMutator;

impl Named for SequenceTailMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceTailMutator")
    }
}

impl<S> Mutator<input::CallSequenceInput, S> for SequenceTailMutator
where
    S: HasRand + HasCorpus<input::CallSequenceInput>,
{
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut input::CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        let count = state.corpus().count();
        if count == 0 {
            return Ok(MutationResult::Skipped);
        }
        let id = state
            .rand_mut()
            .below(NonZeroUsize::new(count).ok_or_else(|| libafl::Error::unknown("non-zero"))?);
        let seq = state
            .corpus()
            .get(CorpusId::from(id))
            .map_err(|e| libafl::Error::unknown(format!("corpus get: {e}")))?
            .borrow()
            .input()
            .clone()
            .ok_or_else(|| libafl::Error::unknown("missing input in corpus"))?;
        if seq.calls.is_empty() {
            return Ok(MutationResult::Skipped);
        }
        let tail_len = state.rand_mut().below(
            NonZeroUsize::new(seq.calls.len()).ok_or_else(|| libafl::Error::unknown("non-zero"))?,
        ) + 1;
        input.calls = seq.calls[seq.calls.len() - tail_len..].to_vec();
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
