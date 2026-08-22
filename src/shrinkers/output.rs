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
