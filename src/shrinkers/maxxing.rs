//! Per-thread shrinker for maxxing-mode results.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use anyhow::{Context, Result};

use crate::corpus::{CorpusConfig, Item, SharedCorpus};
use crate::evm;
use crate::evm::{ExecOutput, Transaction};
use crate::fuzzers::{MaxObjective, SharedMetrics};
use crate::shrinkers::engine::{EngineConfig, ShrinkStrategy, Shrinker};
use crate::shrinkers::{MaxxingShrinkerCorpus, MaxxingShrinkerOutput};

/// Per-shrinker configuration for maxxing mode, configured via a fluent
/// builder API.
#[derive(Clone, Debug)]
pub struct MaxxingShrinkerConfig {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shared_corpus: MaxxingShrinkerCorpus,
    pub shutdown_signal: Arc<AtomicBool>,
    pub objective: Option<MaxObjective>,
    pub max_runs: u64,
    pub timeout: Option<Duration>,
    pub shared_metrics: SharedMetrics,
    pub gas_limit: u64,
    pub caller: Address,
}

impl MaxxingShrinkerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_corpus: MaxxingShrinkerCorpus::new(
                Item::from(vec![]),
                U256::ZERO,
                CorpusConfig::new(""),
                SharedCorpus::new(CorpusConfig::new("")),
            ),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            objective: None,
            max_runs: 0,
            timeout: None,
            shared_metrics: SharedMetrics::new(Vec::new()),
            gas_limit: 12_500_000,
            caller: evm::DEFAULT_DEPLOYER,
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

    /// Set the harness contract address.
    pub fn target_address(mut self, value: Address) -> Self {
        self.target_address = value;
        self
    }

    /// Set the shared shrinker corpus.
    pub fn shared_corpus(mut self, value: MaxxingShrinkerCorpus) -> Self {
        self.shared_corpus = value;
        self
    }

    /// Set the shared shutdown signal.
    pub fn shutdown_signal(mut self, value: Arc<AtomicBool>) -> Self {
        self.shutdown_signal = value;
        self
    }

    /// Set the objective whose value must be preserved while shrinking.
    pub fn objective(mut self, value: MaxObjective) -> Self {
        self.objective = Some(value);
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

    /// Set the gas limit for each shrinker-generated transaction.
    pub fn gas_limit(mut self, value: u64) -> Self {
        self.gas_limit = value;
        self
    }

    /// Set the caller address.
    pub fn caller(mut self, value: Address) -> Self {
        self.caller = value;
        self
    }
}

impl Default for MaxxingShrinkerConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-thread shrinker that minimizes a maxxing result while preserving its
/// value.
///
/// Created via [`MaxxingShrinkerConfig`] and run via
/// [`MaxxingShrinker::run`].
#[derive(Debug)]
pub struct MaxxingShrinker(Shrinker<MaxxingStrategy>);

impl MaxxingShrinker {
    /// Create a new maxxing shrinker with the given config.
    pub fn new(config: MaxxingShrinkerConfig) -> Self {
        let MaxxingShrinkerConfig {
            seed,
            chain,
            target_address,
            shared_corpus,
            shutdown_signal,
            objective,
            max_runs,
            timeout,
            shared_metrics,
            gas_limit,
            caller,
        } = config;
        let strategy = MaxxingStrategy {
            shared_corpus,
            objective: objective.unwrap_or_else(default_objective),
            caller,
            gas_limit,
        };
        Self(Shrinker::new(
            EngineConfig {
                seed,
                chain,
                target_address,
                shutdown_signal,
                max_runs,
                timeout,
                shared_metrics,
            },
            strategy,
        ))
    }

    /// Run the shrinker for up to `max_runs` iterations on a single thread.
    ///
    /// Each iteration draws a mutated copy of the current best item, executes
    /// it followed by the max objective call, and accepts the candidate when it
    /// preserves or improves the stored value and shrinks the sequence.
    pub fn run(self) -> Result<MaxxingShrinkerOutput> {
        self.0.run()
    }
}

/// Maxxing-mode strategy: keep the smallest candidate that preserves or
/// improves the stored objective value.
#[derive(Debug)]
struct MaxxingStrategy {
    shared_corpus: MaxxingShrinkerCorpus,
    objective: MaxObjective,
    caller: Address,
    gas_limit: u64,
}

impl ShrinkStrategy for MaxxingStrategy {
    type Output = MaxxingShrinkerOutput;

    fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        self.shared_corpus.next_item(rng)
    }

    fn sequence(&self, item: &Item, target: Address) -> Vec<Transaction> {
        let mut transactions: Vec<Transaction> = item
            .calls
            .iter()
            .map(|call| call.into_transaction(target))
            .collect();
        transactions.push(
            self.objective
                .transaction(target, self.caller, self.gas_limit),
        );
        transactions
    }

    fn observe(&self, item: Item, exec: &ExecOutput) -> Result<()> {
        let raw_score = self
            .objective
            .decode(exec.results.last().context("max call result missing")?)
            .unwrap_or_default();
        self.shared_corpus.accept(item, raw_score);
        Ok(())
    }

    fn output(self, runs: u64, total_calls: u64, total_gas: u64) -> MaxxingShrinkerOutput {
        MaxxingShrinkerOutput {
            runs,
            total_calls,
            total_gas,
        }
    }
}

fn default_objective() -> MaxObjective {
    MaxObjective::new(Function {
        name: String::from("max_value"),
        inputs: Vec::new(),
        outputs: Vec::new(),
        state_mutability: alloy_json_abi::StateMutability::View,
    })
}
