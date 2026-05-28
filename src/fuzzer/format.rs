//! Formatting utilities for fuzzer output.

use revm::primitives::{Address, Bytes};

use crate::evm;
use crate::fuzzer::FailedAssertion;

/// Format a failed assertion's call sequence as a flat, Medusa-style log.
pub fn format_failure(
    contract: &evm::Contract,
    failure: &FailedAssertion,
    sender: Address,
) -> String {
    let mut lines = Vec::new();
    for (i, tx) in failure.transactions.iter().enumerate() {
        let n = i + 1;

        let block = n as u64;
        let time = n as u64;

        lines.push(format!(
            "{}) {}::{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?})",
            n,
            contract.artifact_id.name,
            format_calldata(&tx.calldata),
            block,
            time,
            u64::MAX,
            sender,
        ));
    }
    lines.join("\n")
}

fn format_calldata(calldata: &Bytes) -> String {
    if calldata.is_empty() {
        return "()".into();
    }
    format!("0x{}", hex::encode(calldata))
}
