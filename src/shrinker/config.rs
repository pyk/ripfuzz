//! Shrinker configuration.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use alloy_primitives::Address;

use crate::corpus::{CorpusConfig, Item, SharedFailedCorpusItem};
use crate::evm;
use crate::fuzzer::SharedMetrics;

/// Per-shrinker configuration configured via a fluent builder API.
#[derive(Clone, Debug)]
pub struct ShrinkerConfig {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shared_failed_corpus: SharedFailedCorpusItem,
    pub shutdown_signal: Arc<AtomicBool>,
    pub max_runs: u64,
    pub timeout: Option<Duration>,
    pub shared_metrics: SharedMetrics,
    pub fail_on_revert: bool,
}

impl ShrinkerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_failed_corpus: SharedFailedCorpusItem::new(
                Item::from(vec![]),
                CorpusConfig::new(""),
            ),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            max_runs: 0,
            timeout: None,
            shared_metrics: SharedMetrics::new(Vec::new()),
            fail_on_revert: false,
        }
    }

    /// Set the RNG seed.
    pub fn seed(mut self, value: u64) -> Self {
        self.seed = value;
        self
    }

    /// Set the chain snapshot.
    pub fn chain(mut self, value: evm::Chain) -> Self {
        self.chain = value;
        self
    }

    /// Set the target contract address.
    pub fn target_address(mut self, value: Address) -> Self {
        self.target_address = value;
        self
    }

    /// Set the shared failed corpus item.
    pub fn shared_failed_item(mut self, value: SharedFailedCorpusItem) -> Self {
        self.shared_failed_corpus = value;
        self
    }

    /// Set the shared shutdown signal.
    pub fn shutdown_signal(mut self, value: Arc<AtomicBool>) -> Self {
        self.shutdown_signal = value;
        self
    }

    /// Set the maximum number of runs.
    pub fn max_runs(mut self, value: u64) -> Self {
        self.max_runs = value;
        self
    }

    /// Set the timeout.
    pub fn timeout(mut self, value: Option<Duration>) -> Self {
        self.timeout = value;
        self
    }

    /// Set the shared metrics.
    pub fn shared_metrics(mut self, value: SharedMetrics) -> Self {
        self.shared_metrics = value;
        self
    }

    /// Set whether any revert should be treated as a failure.
    pub fn fail_on_revert(mut self, value: bool) -> Self {
        self.fail_on_revert = value;
        self
    }
}

impl Default for ShrinkerConfig {
    fn default() -> Self {
        Self::new()
    }
}
