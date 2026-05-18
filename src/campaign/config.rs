//! Campaign configuration.

use std::path::PathBuf;

use alloy_primitives::Address;
use revm::primitives::U256;

/// Configuration for a fuzzing campaign.
#[derive(Debug, Clone)]
pub struct CampaignConfig {
    pub threads: usize,
    pub max_runs: u64,
    pub timeout_secs: Option<u64>,
    pub sequence_length: usize,
    pub seed: u64,
    pub max_block_number_delay: u64,
    pub max_block_timestamp_delay: u64,
    /// Path to the corpus root directory. If set, coverage-guided
    /// persistence is enabled.
    pub corpus_dir: Option<PathBuf>,
    /// Enable the `ffi` cheatcode (allows arbitrary host command execution).
    pub ffi: bool,
    /// Wei value sent with the target contract deployment transaction.
    pub deploy_value: U256,
    /// Account address used to deploy the target contract.
    pub deployer_address: Address,
}

impl Default for CampaignConfig {
    fn default() -> Self {
        Self {
            threads: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            max_runs: 10000,
            timeout_secs: None,
            sequence_length: 32,
            seed: 0,
            max_block_number_delay: 5,
            max_block_timestamp_delay: 5,
            corpus_dir: None,
            ffi: false,
            deploy_value: U256::ZERO,
            deployer_address: crate::chain::init::DEFAULT_DEPLOYER,
        }
    }
}

impl CampaignConfig {
    /// Resolved fuzzer count.
    pub fn fuzzer_count(&self) -> usize {
        self.threads
    }
}
