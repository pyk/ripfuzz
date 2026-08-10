//! Result produced by a single fuzzer thread.
//!
//! `failures` contains only the distinct failed assertions that this thread
//! added to the shared
//! [`SharedFailedAssertions`](crate::fuzzer::SharedFailedAssertions) collector.
//! Use the shared collector for the campaign-wide view.

use crate::fuzzer::FailedAssertion;

/// Result produced by a single fuzzer thread.
#[derive(Debug, Clone)]
pub struct FuzzerOutput {
    pub runs: u64,
    pub failures: Vec<FailedAssertion>,
    pub total_calls: u64,
    pub total_gas: u64,
}
