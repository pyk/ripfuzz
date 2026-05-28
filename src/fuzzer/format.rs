//! Formatting utilities for fuzzer output.

use revm::primitives::Address;

use crate::evm;
use crate::fuzzer::Crash;

/// Format a crash's call sequence as a flat, Medusa-style log.
pub fn format_failure(contract: &evm::Contract, failure: &Crash, sender: Address) -> String {
    let mut lines = Vec::new();
    for (i, call) in failure.call_sequence.iter().enumerate() {
        let n = i + 1;

        let block = n as u64;
        let time = n as u64;

        let func_name = call.function.name.as_str();
        let args = match &call.args {
            alloy_dyn_abi::DynSolValue::Tuple(v) if v.is_empty() => "()".into(),
            alloy_dyn_abi::DynSolValue::Tuple(v) => {
                let args_str = v
                    .iter()
                    .map(format_dyn_value)
                    .collect::<Vec<String>>()
                    .join(", ");
                format!("({})", args_str)
            }
            other => format_dyn_value(other),
        };

        lines.push(format!(
            "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?})",
            n,
            contract.artifact_id.name,
            func_name,
            args,
            block,
            time,
            u64::MAX,
            sender,
        ));
    }
    lines.join("\n")
}

fn format_dyn_value(v: &alloy_dyn_abi::DynSolValue) -> String {
    match v {
        alloy_dyn_abi::DynSolValue::Bool(b) => format!("{}", b),
        alloy_dyn_abi::DynSolValue::Int(i, _) => format!("{}", i),
        alloy_dyn_abi::DynSolValue::Uint(u, _) => format!("{}", u),
        alloy_dyn_abi::DynSolValue::Address(a) => format!("{:?}", a),
        alloy_dyn_abi::DynSolValue::String(s) => format!("\"{}\"", s),
        alloy_dyn_abi::DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        alloy_dyn_abi::DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        _ => format!("{:?}", v),
    }
}
