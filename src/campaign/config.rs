//! Campaign configuration.

/// Configuration for a fuzzing campaign.
#[derive(Debug, Clone)]
pub struct CampaignConfig {
    pub workers: usize,
    pub max_runs: u64,
    pub timeout_secs: u64,
    pub sequence_length: usize,
    pub seed: u64,
    pub max_block_number_delay: u64,
    pub max_block_timestamp_delay: u64,
    pub broker_port: u16,
}

impl Default for CampaignConfig {
    fn default() -> Self {
        Self {
            workers: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            max_runs: 10000,
            timeout_secs: 60,
            sequence_length: 5,
            seed: 0,
            max_block_number_delay: 5,
            max_block_timestamp_delay: 5,
            broker_port: 0,
        }
    }
}

impl CampaignConfig {
    /// Resolved worker count.
    pub fn worker_count(&self) -> usize {
        self.workers
    }
}
