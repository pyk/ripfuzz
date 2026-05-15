use std::borrow::Cow;
use std::num::NonZeroUsize;

use alloy_json_abi::JsonAbi;
use libafl::{
    corpus::Corpus,
    mutators::{MutationResult, Mutator},
    state::HasCorpus,
    state::HasRand,
};
use libafl_bolts::{Named, rands::Rand};

use crate::fuzzer::sequence::{Call, CallSequenceInput};

// ============================================================================
// Sequence-level mutators (no corpus access needed)
// ============================================================================

/// Swap two random calls in the sequence.
#[derive(Debug, Default)]
pub struct SequenceSwapMutator;

impl Named for SequenceSwapMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceSwapMutator")
    }
}

impl<S: HasRand> Mutator<CallSequenceInput, S> for SequenceSwapMutator {
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        if input.calls.len() < 2 {
            return Ok(MutationResult::Skipped);
        }
        let idx1 = state
            .rand_mut()
            .below(NonZeroUsize::new(input.calls.len()).unwrap());
        let idx2 = state
            .rand_mut()
            .below(NonZeroUsize::new(input.calls.len()).unwrap());
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
        _new_corpus_id: Option<libafl::corpus::CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}

/// Insert a new random call at a random position.
#[derive(Debug, Default)]
pub struct SequenceInsertMutator {
    selectors: Vec<[u8; 4]>,
}

impl SequenceInsertMutator {
    pub fn new(selectors: Vec<[u8; 4]>) -> Self {
        Self { selectors }
    }
}

impl Named for SequenceInsertMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceInsertMutator")
    }
}

impl<S: HasRand> Mutator<CallSequenceInput, S> for SequenceInsertMutator {
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        if self.selectors.is_empty() {
            return Ok(MutationResult::Skipped);
        }
        let idx = if input.calls.is_empty() {
            0
        } else {
            state
                .rand_mut()
                .below(NonZeroUsize::new(input.calls.len() + 1).unwrap())
        };
        let sel_idx = state
            .rand_mut()
            .below(NonZeroUsize::new(self.selectors.len()).unwrap());
        let call = Call {
            selector: self.selectors[sel_idx],
            args: vec![0u8; 32 * 3], // up to 3 args of padding
        };
        input.calls.insert(idx, call);
        Ok(MutationResult::Mutated)
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _new_corpus_id: Option<libafl::corpus::CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}

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
        _new_corpus_id: Option<libafl::corpus::CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}

// ============================================================================
// Corpus-level mutators
// ============================================================================

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
            .get(libafl::corpus::CorpusId::from(id1))
            .map_err(|e| libafl::Error::unknown(format!("corpus get: {e}")))?
            .borrow()
            .input()
            .clone()
            .ok_or_else(|| libafl::Error::unknown("missing input in corpus"))?;
        let seq2 = state
            .corpus()
            .get(libafl::corpus::CorpusId::from(id2))
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
        _new_corpus_id: Option<libafl::corpus::CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}

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
            .get(libafl::corpus::CorpusId::from(id1))
            .map_err(|e| libafl::Error::unknown(format!("corpus get: {e}")))?
            .borrow()
            .input()
            .clone()
            .ok_or_else(|| libafl::Error::unknown("missing input in corpus"))?;
        let seq2 = state
            .corpus()
            .get(libafl::corpus::CorpusId::from(id2))
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
        _new_corpus_id: Option<libafl::corpus::CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}

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
            .get(libafl::corpus::CorpusId::from(id))
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
        _new_corpus_id: Option<libafl::corpus::CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}

/// Take the tail of a corpus sequence and keep it, discarding the rest.
#[derive(Debug, Default)]
pub struct SequenceTailMutator;

impl Named for SequenceTailMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceTailMutator")
    }
}

impl<S> Mutator<CallSequenceInput, S> for SequenceTailMutator
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
            .get(libafl::corpus::CorpusId::from(id))
            .map_err(|e| libafl::Error::unknown(format!("corpus get: {e}")))?
            .borrow()
            .input()
            .clone()
            .ok_or_else(|| libafl::Error::unknown("missing input in corpus"))?;
        if seq.calls.is_empty() {
            return Ok(MutationResult::Skipped);
        }
        let tail_len = state
            .rand_mut()
            .below(NonZeroUsize::new(seq.calls.len()).unwrap())
            + 1;
        input.calls = seq.calls[seq.calls.len() - tail_len..].to_vec();
        Ok(MutationResult::Mutated)
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _new_corpus_id: Option<libafl::corpus::CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}

// ============================================================================
// ABI-aware argument mutator
// ============================================================================

/// Mutate the arguments of a random call using ABI type information.
#[derive(Debug)]
pub struct SequenceArgMutator {
    abi: JsonAbi,
}

impl SequenceArgMutator {
    pub fn new(abi: JsonAbi) -> Self {
        Self { abi }
    }
}

impl Named for SequenceArgMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceArgMutator")
    }
}

impl<S: HasRand> Mutator<CallSequenceInput, S> for SequenceArgMutator {
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        if input.calls.is_empty() {
            return Ok(MutationResult::Skipped);
        }
        let call_idx = state
            .rand_mut()
            .below(NonZeroUsize::new(input.calls.len()).unwrap());
        let call = &mut input.calls[call_idx];

        // Find the function in the ABI by selector.
        let func = self
            .abi
            .functions()
            .find(|f| f.selector().as_slice() == &call.selector[..]);

        if let Some(func) = func {
            // ABI-aware mutation: for each argument, apply a type-specific mutation.
            let mut mutated = false;
            for (i, input_def) in func.inputs.iter().enumerate() {
                let start = i * 32;
                let end = start + 32;
                if end > call.args.len() {
                    break;
                }
                let arg_bytes = &mut call.args[start..end];

                // Apply a mutation based on the Solidity type.
                if input_def.ty.starts_with("uint") || input_def.ty.starts_with("int") {
                    // Mutate by adding/subtracting a small random delta.
                    let delta = (state.rand_mut().next() % 1_000) as i64 - 500;
                    let mut val = u128::from_be_bytes([
                        arg_bytes[16],
                        arg_bytes[17],
                        arg_bytes[18],
                        arg_bytes[19],
                        arg_bytes[20],
                        arg_bytes[21],
                        arg_bytes[22],
                        arg_bytes[23],
                        arg_bytes[24],
                        arg_bytes[25],
                        arg_bytes[26],
                        arg_bytes[27],
                        arg_bytes[28],
                        arg_bytes[29],
                        arg_bytes[30],
                        arg_bytes[31],
                    ]);
                    if delta >= 0 {
                        val = val.saturating_add(delta as u128);
                    } else {
                        val = val.saturating_sub((-delta) as u128);
                    }
                    let new_bytes = val.to_be_bytes();
                    arg_bytes[16..32].copy_from_slice(&new_bytes);
                    mutated = true;
                } else if input_def.ty == "bool" {
                    // Flip the last byte.
                    arg_bytes[31] = if arg_bytes[31] == 0 { 1 } else { 0 };
                    mutated = true;
                } else if input_def.ty == "address" {
                    // Randomize the last 20 bytes.
                    let rand = state.rand_mut().next();
                    arg_bytes[12..20].copy_from_slice(&rand.to_be_bytes());
                    mutated = true;
                } else {
                    // Fallback: random byte flip in the word.
                    let byte_idx = state.rand_mut().below(NonZeroUsize::new(32).unwrap());
                    arg_bytes[byte_idx] = state.rand_mut().next() as u8;
                    mutated = true;
                }
            }
            if mutated {
                Ok(MutationResult::Mutated)
            } else {
                Ok(MutationResult::Skipped)
            }
        } else {
            // No ABI info: fall back to random byte mutation.
            if call.args.is_empty() {
                return Ok(MutationResult::Skipped);
            }
            let byte_idx = state
                .rand_mut()
                .below(NonZeroUsize::new(call.args.len()).unwrap());
            call.args[byte_idx] = state.rand_mut().next() as u8;
            Ok(MutationResult::Mutated)
        }
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _new_corpus_id: Option<libafl::corpus::CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}
