//! Shrinker configuration.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use alloy_json_abi::Function;
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
    pub shared_failed_item: SharedFailedCorpusItem,
    pub shutdown_signal: Arc<AtomicBool>,
    pub caller: Address,
    pub invariant_functions: Vec<Function>,
    pub max_runs: u64,
    pub timeout: Option<Duration>,
    pub shared_metrics: SharedMetrics,
}

impl ShrinkerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_failed_item: SharedFailedCorpusItem::new(
                Item::from(vec![]),
                CorpusConfig::new(""),
            ),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            caller: evm::DEFAULT_DEPLOYER,
            invariant_functions: Vec::new(),
            max_runs: 0,
            timeout: None,
            shared_metrics: SharedMetrics::new(Vec::new()),
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
        self.shared_failed_item = value;
        self
    }

    /// Set the shared shutdown signal.
    pub fn shutdown_signal(mut self, value: Arc<AtomicBool>) -> Self {
        self.shutdown_signal = value;
        self
    }

    /// Set the invariant functions to append after each sequence.
    pub fn invariant_functions(mut self, value: Vec<Function>) -> Self {
        self.invariant_functions = value;
        self
    }

    /// Set the caller address.
    pub fn caller(mut self, value: Address) -> Self {
        self.caller = value;
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
}

impl Default for ShrinkerConfig {
    fn default() -> Self {
        Self::new()
    }
}
