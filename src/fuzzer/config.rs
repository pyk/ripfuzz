use clap::Args;

/// Fuzzer configuration passed via CLI.
#[derive(Debug, Clone, Args)]
pub struct FuzzConfig {
    /// Maximum number of fuzzing iterations.
    #[arg(long = "fuzz-iters", default_value = "10000")]
    pub max_iters: u64,

    /// Timeout in seconds for the entire fuzzing campaign.
    #[arg(long = "fuzz-timeout", default_value = "60")]
    pub timeout_secs: u64,

    /// Maximum number of calls in a generated sequence.
    #[arg(long = "fuzz-seq-len", default_value = "5")]
    pub sequence_length: usize,

    /// Random seed for reproducibility.
    #[arg(long = "fuzz-seed", default_value = "0")]
    pub seed: u64,

    /// Maximum block number delay between calls.
    #[arg(long = "max-block-delay", default_value = "5")]
    pub max_block_number_delay: u64,

    /// Maximum block timestamp delay between calls.
    #[arg(long = "max-time-delay", default_value = "5")]
    pub max_block_timestamp_delay: u64,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            max_iters: 10000,
            timeout_secs: 60,
            sequence_length: 5,
            seed: 0,
            max_block_number_delay: 5,
            max_block_timestamp_delay: 5,
        }
    }
}
