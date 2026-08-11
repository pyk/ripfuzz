//! Results produced by fuzzer threads.

/// Result produced by a single invariant fuzzer thread.
#[derive(Debug, Clone)]
pub struct InvariantFuzzerOutput {
    pub runs: u64,
    pub total_calls: u64,
    pub total_gas: u64,
}

/// Result produced by a single maxxing fuzzer thread.
#[derive(Debug, Clone)]
pub struct MaxxingFuzzerOutput {
    pub runs: u64,
    pub total_calls: u64,
    pub total_gas: u64,
}
