//! Fuzzer configuration.

use std::path::PathBuf;
use std::time::Duration;

use alloy_json_abi::Function;
use alloy_primitives::Address;

use crate::evm;
use crate::evm::coverage::SharedCoverage;
use crate::fuzzer::corpus::SharedCorpus;
use crate::fuzzer::metrics::SharedMetrics;

/// Per-fuzzer configuration configured via a fluent builder API.
#[derive(Clone, Debug)]
pub struct Config {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shared_corpus: SharedCorpus,
    pub shared_coverage: SharedCoverage,
    pub shared_metrics: SharedMetrics,
    pub caller: Address,
    pub invariant_functions: Vec<Function>,
    pub max_runs: u64,
    pub timeout: Option<Duration>,
}

impl Config {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_corpus: SharedCorpus::new(crate::fuzzer::corpus::Config::new(PathBuf::new())),
            shared_coverage: SharedCoverage::new(),
            shared_metrics: SharedMetrics::new(),
            caller: evm::chain::DEFAULT_DEPLOYER,
            invariant_functions: Vec::new(),
            max_runs: 0,
            timeout: None,
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

    /// Set the shared corpus.
    pub fn shared_corpus(mut self, value: SharedCorpus) -> Self {
        self.shared_corpus = value;
        self
    }

    /// Set the shared coverage map.
    pub fn shared_coverage(mut self, value: SharedCoverage) -> Self {
        self.shared_coverage = value;
        self
    }

    /// Set the shared metrics.
    pub fn shared_metrics(mut self, value: SharedMetrics) -> Self {
        self.shared_metrics = value;
        self
    }

    /// Set the invariant functions to append after each corpus sequence.
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
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}
