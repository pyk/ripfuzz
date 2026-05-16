//! Corpus mutator that interleaves two sequences.

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

/// Interleave two corpus sequences.
#[derive(Debug, Default)]
pub struct SequenceInterleaveMutator;

impl Named for SequenceInterleaveMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceInterleaveMutator")
    }
}

impl<S> Mutator<input::CallSequenceInput, S> for SequenceInterleaveMutator
where
    S: HasRand + HasCorpus<input::CallSequenceInput>,
{
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut input::CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        let count = state.corpus().count();
        if count < 2 {
            return Ok(MutationResult::Skipped);
        }

        let id1 = state
            .rand_mut()
            .below(NonZeroUsize::new(count).ok_or_else(|| libafl::Error::unknown("non-zero"))?);
        let id2 = state
            .rand_mut()
            .below(NonZeroUsize::new(count).ok_or_else(|| libafl::Error::unknown("non-zero"))?);

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

        let take1 = state.rand_mut().below(
            NonZeroUsize::new(seq1.calls.len() + 1)
                .ok_or_else(|| libafl::Error::unknown("non-zero"))?,
        );
        let take2 = state.rand_mut().below(
            NonZeroUsize::new(seq2.calls.len() + 1)
                .ok_or_else(|| libafl::Error::unknown("non-zero"))?,
        );

        let slice1 = &seq1.calls[..take1];
        let slice2 = &seq2.calls[..take2];

        let calls1 = slice1.to_vec();
        let calls2 = slice2.to_vec();
        let mut iter1 = calls1.into_iter();
        let mut iter2 = calls2.into_iter();
        let mut new_calls = Vec::with_capacity(slice1.len() + slice2.len());
        loop {
            match (iter1.next(), iter2.next()) {
                (Some(a), Some(b)) => {
                    new_calls.push(a);
                    new_calls.push(b);
                }
                (Some(a), None) => new_calls.push(a),
                (None, Some(b)) => new_calls.push(b),
                (None, None) => break,
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
