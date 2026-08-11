//! Results produced by fuzzer threads.
//!
//! `failures` contains only the distinct failed assertions that this thread
//! added to the shared
//! [`SharedFailedAssertions`](crate::fuzzers::SharedFailedAssertions) collector.
//! Use the shared collector for the campaign-wide view.

use crate::fuzzers::FailedAssertion;

/// Result produced by a single invariant fuzzer thread.
#[derive(Debug, Clone)]
pub struct InvariantFuzzerOutput {
    pub runs: u64,
    pub failures: Vec<FailedAssertion>,
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
