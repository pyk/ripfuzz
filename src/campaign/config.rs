//! Campaign configuration.

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
        }
    }
}

impl CampaignConfig {
    /// Resolved fuzzer count.
    pub fn fuzzer_count(&self) -> usize {
        self.threads
    }
}
