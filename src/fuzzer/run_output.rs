//! Result produced by a single fuzzer thread.

use crate::fuzzer::failed_assertion::FailedAssertion;

/// Result produced by a single fuzzer thread.
#[derive(Debug, Clone)]
pub struct RunOutput {
    pub runs: u64,
    pub failures: Vec<FailedAssertion>,
    pub total_calls: u64,
    pub total_gas: u64,
}
