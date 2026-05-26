//! Sequence mutator that inserts a new random call.

use alloy_dyn_abi::{DynSolValue, Specifier};
use alloy_json_abi::Function;

use crate::fuzzer::corpus::Call;
use crate::fuzzer::corpus::call::default_dyn_value;
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Insert a new random call at a random position.
#[derive(Debug, Default)]
pub struct SequenceInsertMutator {
    functions: Vec<Function>,
}

impl SequenceInsertMutator {
    pub fn new(functions: Vec<Function>) -> Self {
        Self { functions }
    }
}

impl Mutator for SequenceInsertMutator {
    fn mutate(&self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        if self.functions.is_empty() {
            return MutationResult::Skipped;
        }
        let idx = if calls.is_empty() {
            0
        } else {
            rng.usize(0..calls.len() + 1)
        };
        let func_idx = rng.usize(0..self.functions.len());
        let func = &self.functions[func_idx];

        let values: Vec<DynSolValue> = func
            .inputs
            .iter()
            .filter_map(|p| p.resolve().ok())
            .map(|ty| default_dyn_value(&ty))
            .collect();
        let call = Call {
            function: func.clone(),
            values: DynSolValue::Tuple(values),
        };
        calls.insert(idx, call);
        MutationResult::Mutated
    }
}
