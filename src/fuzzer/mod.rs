//! Coverage-guided fuzzing engine for Solidity smart contracts.

use alloy_dyn_abi::{DynSolType, DynSolValue};

use crate::contract;

pub mod mutators;
pub mod sequence;

/// A single property failure discovered during fuzzing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PropertyFailure {
    pub property_name: String,
    pub property_selector: [u8; 4],
    pub call_sequence: sequence::CallSequenceInput,
    /// Per-call block number / timestamp captured during execution.
    pub call_meta: Vec<crate::evm::CallMeta>,
}

/// Format a property failure's call sequence as a flat, Medusa-style log.
pub fn format_failure(artifact: &contract::ContractArtifact, failure: &PropertyFailure) -> String {
    let mut lines = Vec::new();
    for (i, call) in failure.call_sequence.calls.iter().enumerate() {
        let n = i + 1;

        let block = failure
            .call_meta
            .get(i)
            .map(|m| m.block_number)
            .unwrap_or(n as u64);
        let time = failure
            .call_meta
            .get(i)
            .map(|m| m.block_timestamp)
            .unwrap_or(n as u64);

        let func = artifact
            .abi
            .functions()
            .find(|f| f.selector().as_slice() == call.selector);

        let func_name = if let Some(f) = func {
            f.name.to_owned()
        } else {
            format!("0x{}", hex::encode(call.selector))
        };

        let mut delay_suffix = String::new();
        if call.block_number_delay != 0 {
            delay_suffix.push_str(&format!(", block_number_delay={}", call.block_number_delay));
        }
        if call.block_timestamp_delay != 0 {
            delay_suffix.push_str(&format!(
                ", block_timestamp_delay={}",
                call.block_timestamp_delay
            ));
        }

        let args = if let Some(func_abi) = func {
            if call.args.is_empty() {
                "()".into()
            } else {
                let types_result = func_abi
                    .inputs
                    .iter()
                    .map(|p| p.selector_type().parse::<DynSolType>())
                    .collect();
                let Ok(types) = types_result else {
                    let raw = format!("(0x{})", hex::encode(&call.args));
                    lines.push(format!(
                        "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
                        n,
                        artifact.contract_name,
                        func_name,
                        raw,
                        block,
                        time,
                        crate::evm::GAS_LIMIT,
                        crate::evm::CALLER,
                        delay_suffix,
                    ));
                    continue;
                };

                let tuple = DynSolType::Tuple(types);
                let Ok(decoded) = tuple.abi_decode_params(&call.args) else {
                    let raw = format!("(0x{})", hex::encode(&call.args));
                    lines.push(format!(
                        "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
                        n,
                        artifact.contract_name,
                        func_name,
                        raw,
                        block,
                        time,
                        crate::evm::GAS_LIMIT,
                        crate::evm::CALLER,
                        delay_suffix,
                    ));
                    continue;
                };

                let values = match decoded {
                    DynSolValue::Tuple(v) => v,
                    other => vec![other],
                };

                let args_str = values
                    .iter()
                    .map(format_dyn_value)
                    .collect::<Vec<String>>()
                    .join(", ");

                format!("({})", args_str)
            }
        } else {
            format!("0x{}", hex::encode(&call.args))
        };

        lines.push(format!(
            "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
            n,
            artifact.contract_name,
            func_name,
            args,
            block,
            time,
            crate::evm::GAS_LIMIT,
            crate::evm::CALLER,
            delay_suffix,
        ));
    }
    lines.join("\n")
}

/// Format a single decoded Solidity value for display.
fn format_dyn_value(v: &alloy_dyn_abi::DynSolValue) -> String {
    match v {
        DynSolValue::Bool(b) => format!("{}", b),
        DynSolValue::Int(i, _) => format!("{}", i),
        DynSolValue::Uint(u, _) => format!("{}", u),
        DynSolValue::Address(a) => format!("{:?}", a),
        DynSolValue::String(s) => format!("\"{}\"", s),
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        _ => format!("{:?}", v),
    }
}

/// Build seed inputs from the contract ABI.
pub fn build_seeds(
    artifact: &contract::ContractArtifact,
    max_len: usize,
) -> Vec<sequence::CallSequenceInput> {
    let mut seeds = Vec::new();

    // Single-call seeds for every ABI function.
    for func in artifact.abi.functions() {
        let selector: [u8; 4] = func.selector().into();
        let call = sequence::Call {
            selector,
            args: vec![0u8; func.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        };
        seeds.push(sequence::CallSequenceInput::single(call));
    }

    // Combined seed with all non-view/pure action functions in ABI order.
    let action_calls: Vec<sequence::Call> = artifact
        .abi
        .functions()
        .filter(|f| {
            !matches!(
                f.state_mutability,
                alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
            )
        })
        .map(|f| sequence::Call {
            selector: f.selector().into(),
            args: vec![0u8; f.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        })
        .collect();

    if !action_calls.is_empty() {
        let mut combined = sequence::CallSequenceInput::new();
        combined.calls = action_calls.clone();
        seeds.push(combined);
    }

    // Permutation seeds for action functions (up to max_len).
    let n = action_calls.len();
    if n > 0 && n <= max_len {
        let mut queue = std::collections::VecDeque::new();
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
            let mut seq = sequence::CallSequenceInput::new();
            for &i in &perm {
                seq.calls.push(action_calls[i].replicate());
            }
            seeds.push(seq);
        }
    }

    seeds
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::campaign::{Campaign, CampaignConfig};
    use crate::contract;
    use crate::evm;
    use crate::fuzzer::PropertyFailure;
    use crate::fuzzer::sequence;

    #[test]
    fn deployment_reports_constructor_revert_reason() {
        let _artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ConstructorRevert.sol"),
        )
        .unwrap();

        let mut config = CampaignConfig::default();
        config.workers = 1;
        let err = Campaign::for_target(
            Path::new("test/ConstructorRevert.sol"),
            Path::new("fixtures/basic-target"),
        )
        .with_config(config)
        .build()
        .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ConstructorRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_complex_constructor_trace() {
        let _artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ComplexConstructorRevert.sol"),
        )
        .unwrap();

        let mut config = CampaignConfig::default();
        config.workers = 1;
        let err = Campaign::for_target(
            Path::new("test/ComplexConstructorRevert.sol"),
            Path::new("fixtures/basic-target"),
        )
        .with_config(config)
        .build()
        .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ComplexConstructorRevertOutput.txt")
                .unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_set_up_revert_trace() {
        let _artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/SetupRevert.sol"),
        )
        .unwrap();

        let mut config = CampaignConfig::default();
        config.workers = 1;
        let err = Campaign::for_target(
            Path::new("test/SetupRevert.sol"),
            Path::new("fixtures/basic-target"),
        )
        .with_config(config)
        .build()
        .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/SetupRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn catches_l1_simple_knob_dragon() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
        .unwrap();

        assert!(
            !artifact.properties.is_empty(),
            "property_caught() should be discovered as a property"
        );

        let mut config = CampaignConfig::default();
        config.workers = 1;
        config.max_iters = 10_000;
        let campaign = Campaign::for_target(
            Path::new("src/L1SimpleKnob.sol"),
            Path::new("fixtures/challenges"),
        )
        .with_config(config)
        .build()
        .unwrap();
        let result = campaign.run().unwrap();

        assert!(
            !result.failures.is_empty(),
            "raptor should find at least one property failure (dragon caught)"
        );
    }

    #[test]
    fn format_failure_uses_block_number_and_timestamp_labels() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
        .unwrap();

        let calls = vec![
            sequence::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
            },
            sequence::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 3,
                block_timestamp_delay: 4,
            },
            sequence::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
            },
        ];

        let failure = PropertyFailure {
            property_name: "property_caught".into(),
            property_selector: [0; 4],
            call_sequence: sequence::CallSequenceInput { calls },
            call_meta: vec![
                evm::CallMeta {
                    block_number: 0,
                    block_timestamp: 0,
                },
                evm::CallMeta {
                    block_number: 3,
                    block_timestamp: 4,
                },
                evm::CallMeta {
                    block_number: 4,
                    block_timestamp: 5,
                },
            ],
        };

        let output = crate::fuzzer::format_failure(&artifact, &failure);
        assert!(
            output.contains("block_number="),
            "output should use block_number label:\n{}",
            output
        );
        assert!(
            output.contains("block_timestamp="),
            "output should use block_timestamp label:\n{}",
            output
        );
        assert!(
            !output.contains("block=0") && !output.contains("block=3"),
            "output should not use old block= label:\n{}",
            output
        );
        assert!(
            !output.contains("time=1") && !output.contains("time=5"),
            "output should not use old time= label:\n{}",
            output
        );
        assert!(
            output.contains("block_number_delay=3"),
            "output should show block_number_delay:\n{}",
            output
        );
        assert!(
            output.contains("block_timestamp_delay=4"),
            "output should show block_timestamp_delay:\n{}",
            output
        );
    }
}
