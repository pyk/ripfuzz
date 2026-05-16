//! Seed input generation from the contract ABI.

use std::collections::VecDeque;

use crate::contract::ContractArtifact;
use crate::corpus;

/// Build seed inputs from the contract ABI.
pub fn build_seeds(artifact: &ContractArtifact, max_len: usize) -> Vec<corpus::CallSequenceInput> {
    let mut seeds = Vec::new();
    let mut action_calls = Vec::new();

    let funcs: Vec<alloy_json_abi::Function> = artifact.abi.functions().cloned().collect();
    for func in funcs {
        let is_action = !matches!(
            func.state_mutability,
            alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
        );

        let signature = func.signature();
        let call = corpus::Call {
            selector: func.selector().into(),
            args: vec![0u8; func.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: func.name,
            method_signature: signature,
            ..Default::default()
        };

        if is_action {
            action_calls.push(call.replicate());
        }
        seeds.push(corpus::CallSequenceInput::single(call));
    }

    if !action_calls.is_empty() {
        let mut combined = corpus::CallSequenceInput::new();
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
            let mut seq = corpus::CallSequenceInput::new();
            for &i in &perm {
                seq.calls.push(action_calls[i].replicate());
            }
            seeds.push(seq);
        }
    }

    seeds
}
