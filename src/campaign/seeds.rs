//! Seed input generation from the contract ABI.

use std::collections::VecDeque;

use crate::contract::ContractArtifact;
use crate::corpus::{Call, CorpusItem};

/// Build seed inputs from the contract ABI.
pub fn build_seeds(artifact: &ContractArtifact, max_len: usize) -> Vec<CorpusItem> {
    let mut seeds = Vec::new();
    let mut action_calls = Vec::new();

    let funcs: Vec<alloy_json_abi::Function> = artifact.abi.functions().cloned().collect();
    for func in funcs {
        let is_action = !matches!(
            func.state_mutability,
            alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
        );

        let signature = func.signature();
        let call = Call {
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
        seeds.push(CorpusItem::new(vec![call]));
    }

    if !action_calls.is_empty() {
        seeds.push(CorpusItem::new(action_calls.clone()));
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
            let mut calls = Vec::new();
            for &i in &perm {
                calls.push(action_calls[i].replicate());
            }
            seeds.push(CorpusItem::new(calls));
        }
    }

    seeds
}
