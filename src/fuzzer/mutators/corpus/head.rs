use std::borrow::Cow;
use std::num::NonZeroUsize;

use libafl::{
    corpus::{Corpus, CorpusId},
    mutators::{MutationResult, Mutator},
    state::{HasCorpus, HasRand},
};
use libafl_bolts::Named;
use libafl_bolts::rands::Rand;

use crate::fuzzer::sequence::CallSequenceInput;

/// Take the head of a corpus sequence and keep it, discarding the rest.
#[derive(Debug, Default)]
pub struct SequenceHeadMutator;

impl Named for SequenceHeadMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceHeadMutator")
    }
}

impl<S> Mutator<CallSequenceInput, S> for SequenceHeadMutator
where
    S: HasRand + HasCorpus<CallSequenceInput>,
{
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        let count = state.corpus().count();
        if count == 0 {
            return Ok(MutationResult::Skipped);
        }
        let id = state.rand_mut().below(NonZeroUsize::new(count).unwrap());
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
        let head_len = state
            .rand_mut()
            .below(NonZeroUsize::new(seq.calls.len()).unwrap())
            + 1;
        input.calls = seq.calls[..head_len].to_vec();
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
