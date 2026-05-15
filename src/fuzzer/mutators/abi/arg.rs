use std::borrow::Cow;
use std::num::NonZeroUsize;

use alloy_json_abi::JsonAbi;
use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    state::HasRand,
};
use libafl_bolts::{Named, rands::Rand};

use crate::fuzzer::sequence::CallSequenceInput;

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
        _new_corpus_id: Option<CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}
