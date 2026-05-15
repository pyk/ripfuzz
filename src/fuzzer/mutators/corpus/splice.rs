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

/// Splice two corpus sequences: take the head from one and tail from another.
#[derive(Debug, Default)]
pub struct SequenceSpliceMutator;

impl Named for SequenceSpliceMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceSpliceMutator")
    }
}

impl<S> Mutator<CallSequenceInput, S> for SequenceSpliceMutator
where
    S: HasRand + HasCorpus<CallSequenceInput>,
{
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        let count = state.corpus().count();
        if count < 2 {
            return Ok(MutationResult::Skipped);
        }

        let id1 = state.rand_mut().below(NonZeroUsize::new(count).unwrap());
        let id2 = state.rand_mut().below(NonZeroUsize::new(count).unwrap());

        let seq1 = state
            .corpus()
            .get(CorpusId::from(id1))
            .map_err(|e| libafl::Error::unknown(format!("corpus get: {e}")))?
            .borrow()
            .input()
            .clone()
            .ok_or_else(|| libafl::Error::unknown("missing input in corpus"))?;
        let seq2 = state
            .corpus()
            .get(CorpusId::from(id2))
            .map_err(|e| libafl::Error::unknown(format!("corpus get: {e}")))?
            .borrow()
            .input()
            .clone()
            .ok_or_else(|| libafl::Error::unknown("missing input in corpus"))?;

        if seq1.calls.is_empty() || seq2.calls.is_empty() {
            return Ok(MutationResult::Skipped);
        }

        let head_len = state
            .rand_mut()
            .below(NonZeroUsize::new(seq1.calls.len()).unwrap())
            + 1;
        let tail_len = state
            .rand_mut()
            .below(NonZeroUsize::new(seq2.calls.len() + 1).unwrap());

        let mut new_calls = seq1.calls[..head_len].to_vec();
        new_calls.extend_from_slice(&seq2.calls[seq2.calls.len() - tail_len..]);
        input.calls = new_calls;
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
