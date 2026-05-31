//! Per-thread shrinker that minimizes a failing corpus item.
//!
//! [`Shrinker`](Shrinker) draws a mutated copy of the current smallest failing
//! item, executes it on a fresh chain clone, and replaces the shared item if
//! the mutated sequence is still failing and strictly smaller.
//!
//! [`Shrinker`](Shrinker) is configured via [`Config`](Config)
//! and runs directly on a cloned chain.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::Result;
use tracing::instrument;

use crate::corpus::{Call, Item, SharedFailedCorpusItem};
use crate::evm;
use crate::evm::Transaction;
use crate::fuzzer::SharedMetrics;

/// Result produced by a single shrinker thread.
#[derive(Debug, Clone)]
pub struct ShrinkerOutput {
    pub runs: u64,
    pub total_calls: u64,
    pub total_gas: u64,
}

/// Per-thread shrinker that executes mutated call sequences and keeps the
/// smallest item that still triggers a failed assertion.
///
/// Created via [`Shrinker::new`] and run via [`Shrinker::run`].
#[derive(Debug)]
pub struct Shrinker {
    chain: evm::Chain,
    target_address: Address,
    shared_failed_item: SharedFailedCorpusItem,
    shutdown_signal: Arc<AtomicBool>,
    caller: Address,
    invariant_functions: Vec<Function>,
    max_runs: u64,
    timeout: Option<Duration>,
    shared_metrics: SharedMetrics,
    rng: fastrand::Rng,
}

impl Shrinker {
    /// Create a new shrinker with the given config.
    pub fn new(config: Config) -> Self {
        Self {
            chain: config.chain,
            target_address: config.target_address,
            shared_failed_item: config.shared_failed_item,
            shutdown_signal: config.shutdown_signal,
            caller: config.caller,
            invariant_functions: config.invariant_functions,
            max_runs: config.max_runs,
            timeout: config.timeout,
            shared_metrics: config.shared_metrics,
            rng: fastrand::Rng::with_seed(config.seed),
        }
    }

    /// Run the shrinker for up to `max_runs` iterations on a single thread.
    ///
    /// Each iteration draws a mutated copy of the current smallest failing item,
    /// executes it on a fresh chain clone, and replaces the shared item if the
    /// mutated sequence is still failing and strictly smaller.
    #[instrument(skip(self), fields(max_runs = self.max_runs))]
    pub fn run(mut self) -> Result<ShrinkerOutput> {
        let start = Instant::now();
        let mut runs = 0u64;
        let mut total_calls = 0u64;
        let mut total_gas = 0u64;

        let invariant_calls: Vec<Call> = self
            .invariant_functions
            .iter()
            // checkrs: allow(clone_in_iterator)
            .map(|func| Call {
                function: func.clone(),
                args: DynSolValue::Tuple(vec![]),
                value: None,
                caller: self.caller,
            })
            .collect();

        for _ in 0..self.max_runs {
            if self.shutdown_signal.load(Ordering::Relaxed) {
                break;
            }
            let should_break = match self.timeout {
                Some(t) => start.elapsed() > t,
                None => false,
            };
            if should_break {
                break;
            }

            let item = self.shared_failed_item.next_item(&mut self.rng);
            // checkrs: allow(clone_in_loops)
            let mut fresh_chain = self.chain.clone();
            let transactions: Vec<Transaction> = item
                .calls
                .iter()
                .chain(invariant_calls.iter())
                .map(|call| call.into_transaction(self.target_address))
                .collect();
            let calls_count = transactions.len();

            let exec = fresh_chain.exec(&transactions)?;
            let gas_sum = exec.results.iter().map(|r| r.gas_used).sum::<u64>();

            total_calls += calls_count as u64;
            total_gas += gas_sum;
            self.shared_metrics.record(calls_count as u64, gas_sum);
            runs += 1;

            if !exec.panic_transactions.is_empty() {
                self.shared_failed_item.replace_item(item);
            }
        }

        Ok(ShrinkerOutput {
            runs,
            total_calls,
            total_gas,
        })
    }
}

/// Per-shrinker configuration configured via a fluent builder API.
#[derive(Clone, Debug)]
pub struct Config {
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

impl Config {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_failed_item: SharedFailedCorpusItem::new(
                Item::from(vec![]),
                crate::corpus::Config::new(""),
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

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}
