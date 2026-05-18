//! Campaign result aggregation.

use alloy_primitives::Address;

use crate::corpus::CoverageMap;
use crate::fuzzer::Crash;

/// The aggregated output of a fuzzing campaign.
#[derive(Debug)]
pub struct CampaignResult {
    pub runs: u64,
    pub failures: Vec<Crash>,
    /// Total individual calls executed across all fuzzers.
    pub total_calls: u64,
    /// Total gas consumed across all calls.
    pub total_gas: u64,
    /// Actual wall-clock elapsed time in seconds.
    pub elapsed_secs: f64,
    /// Final global coverage map after all fuzzers finish.
    pub coverage: CoverageMap,
    /// Account address used to deploy the target contract.
    pub deployer_address: Address,
    /// Maximum gas that can be consumed in a single block.
    pub block_gas_limit: u64,
    /// Maximum gas sent with each fuzzer-generated transaction.
    pub tx_gas_limit: u64,
}
