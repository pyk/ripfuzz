//! Fuzzer configuration.

/// Per-fuzzer configuration.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub seed: u64,
    pub sequence_length: usize,
    pub max_block_number_delay: u64,
    pub max_block_timestamp_delay: u64,
}
