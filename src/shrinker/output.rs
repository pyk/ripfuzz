//! Result produced by a single shrinker thread.

/// Result produced by a single shrinker thread.
#[derive(Debug, Clone)]
pub struct ShrinkerOutput {
    pub runs: u64,
    pub total_calls: u64,
    pub total_gas: u64,
}
