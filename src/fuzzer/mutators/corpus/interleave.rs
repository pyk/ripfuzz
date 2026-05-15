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

/// Interleave two corpus sequences.
#[derive(Debug, Default)]
pub struct SequenceInterleaveMutator;

impl Named for SequenceInterleaveMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceInterleaveMutator")
    }
}

impl<S> Mutator<CallSequenceInput, S> for SequenceInterleaveMutator
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

        let take1 = state
            .rand_mut()
            .below(NonZeroUsize::new(seq1.calls.len() + 1).unwrap());
        let take2 = state
            .rand_mut()
            .below(NonZeroUsize::new(seq2.calls.len() + 1).unwrap());

        let slice1 = &seq1.calls[..take1];
        let slice2 = &seq2.calls[..take2];

        let mut new_calls = Vec::new();
        let max_len = slice1.len().max(slice2.len());
        for i in 0..max_len {
            if i < slice1.len() {
                new_calls.push(slice1[i].clone());
            }
            if i < slice2.len() {
                new_calls.push(slice2[i].clone());
            }
        }
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
