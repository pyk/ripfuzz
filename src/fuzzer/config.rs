//! Fuzzer configuration.

/// Per-fuzzer configuration.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub seed: u64,
    pub sequence_length: usize,
}
