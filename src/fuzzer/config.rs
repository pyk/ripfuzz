//! Fuzzer configuration.

/// Configuration specific to a single fuzzer instance.
#[derive(Debug, Clone, Copy)]
pub struct FuzzerConfig {
    pub seed: u64,
    pub sequence_length: usize,
    pub max_block_number_delay: u64,
    pub max_block_timestamp_delay: u64,
}
