//! Seed input generation from the contract ABI.

use std::collections::VecDeque;

use crate::campaign::input;
use crate::contract::ContractArtifact;

/// Build seed inputs from the contract ABI.
pub fn build_seeds(artifact: &ContractArtifact, max_len: usize) -> Vec<input::CallSequenceInput> {
    let mut seeds = Vec::new();

    // Single-call seeds for every ABI function.
    for func in artifact.abi.functions() {
        let selector: [u8; 4] = func.selector().into();
        let call = input::Call {
            selector,
            args: vec![0u8; func.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        };
        seeds.push(input::CallSequenceInput::single(call));
    }

    // Combined seed with all non-view/pure action functions in ABI order.
    let action_calls: Vec<input::Call> = artifact
        .abi
        .functions()
        .filter(|f| {
            !matches!(
                f.state_mutability,
                alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
            )
        })
        .map(|f| input::Call {
            selector: f.selector().into(),
            args: vec![0u8; f.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        })
        .collect();

    if !action_calls.is_empty() {
        let mut combined = input::CallSequenceInput::new();
        combined.calls = action_calls.clone();
        seeds.push(combined);
    }

    // Permutation seeds for action functions (up to max_len).
    let n = action_calls.len();
    if n > 0 && n <= max_len {
        let mut queue = VecDeque::new();
        queue.push_back(Vec::new());
        let mut permutations = Vec::new();
        while let Some(prefix) = queue.pop_front() {
            if prefix.len() == n {
                permutations.push(prefix);
                continue;
            }
            for (idx, _call) in action_calls.iter().enumerate() {
                let already_in_prefix = prefix.contains(&idx);
                if !already_in_prefix {
                    let mut next = prefix.to_vec();
                    next.push(idx);
                    queue.push_back(next);
                }
            }
        }
        for perm in permutations {
            let mut seq = input::CallSequenceInput::new();
            for &i in &perm {
                seq.calls.push(action_calls[i].replicate());
            }
            seeds.push(seq);
        }
    }

    seeds
}
