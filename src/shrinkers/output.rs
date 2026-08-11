//! Results produced by shrinker threads and the final maxxing result.

use alloy_primitives::U256;

use crate::corpus::Item;
use crate::fuzzers::MaxObjective;

/// Result produced by a single invariant shrinker thread.
#[derive(Debug, Clone)]
pub struct InvariantShrinkerOutput {
    pub runs: u64,
    pub total_calls: u64,
    pub total_gas: u64,
}

/// Result produced by a single maxxing shrinker thread.
#[derive(Debug, Clone)]
pub struct MaxxingShrinkerOutput {
    pub runs: u64,
    pub total_calls: u64,
    pub total_gas: u64,
}

/// Final result for one max objective after shrinking.
#[derive(Debug, Clone)]
pub struct MaxxingResult {
    pub objective: MaxObjective,
    pub value: U256,
    pub item: Item,
}

impl MaxxingResult {
    /// Format the call sequence that produced the max value.
    pub fn format_call_sequence(&self) -> String {
        let mut lines = Vec::new();
        for (i, call) in self.item.calls.iter().enumerate() {
            lines.push(format!(
                "    {}. {}({})",
                i + 1,
                call.function.name,
                call.args_json()
            ));
        }
        lines.join("\n")
    }
}
