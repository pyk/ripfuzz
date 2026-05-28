//! Per-thread fuzzer that executes call sequences and reports results.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::Instant;

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};
use tracing::{info, instrument};

use crate::evm;
use crate::evm::chain::Transaction;
use crate::evm::coverage::SharedCoverage;
use crate::fuzzer::config::Config;
use crate::fuzzer::corpus::{Call, SharedCorpus};
use crate::fuzzer::metrics::SharedMetrics;

/// Result produced by a single fuzzer thread.
#[derive(Debug, Clone)]
pub struct RunOutput {
    pub runs: u64,
    pub failures: Vec<FailedAssertion>,
    pub total_calls: u64,
    pub total_gas: u64,
}

/// A single failed assertion (assert panic) discovered during fuzzing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FailedAssertion {
    pub transactions: Vec<Transaction>,
}

/// Per-thread fuzzer that executes call sequences and reports results.
///
/// Created via [`Fuzzer::new`] and run via [`Fuzzer::run`].
#[derive(Debug)]
pub struct Fuzzer {
    chain: evm::Chain,
    target_address: Address,
    shared_corpus: SharedCorpus,
    shared_coverage: SharedCoverage,
    shared_metrics: SharedMetrics,
    shutdown_signal: Arc<AtomicBool>,
    caller: Address,
    invariant_functions: Vec<Function>,
    max_runs: u64,
    timeout: Option<Duration>,
    rng: fastrand::Rng,
}

impl Fuzzer {
    /// Create a new fuzzer with the given config.
    pub fn new(config: Config) -> Self {
        Self {
            chain: config.chain,
            target_address: config.target_address,
            shared_corpus: config.shared_corpus,
            shared_coverage: config.shared_coverage,
            shared_metrics: config.shared_metrics,
            shutdown_signal: config.shutdown_signal,
            caller: config.caller,
            invariant_functions: config.invariant_functions,
            max_runs: config.max_runs,
            timeout: config.timeout,
            rng: fastrand::Rng::with_seed(config.seed),
        }
    }
}

impl Fuzzer {
    /// Run the fuzzer for up to `max_runs` iterations on a single thread.
    ///
    /// The fuzzer loop uses the shared corpus for mutation and the shared
    /// metrics for counters. It stops early if `timeout` is reached.
    #[instrument(skip(self), fields(max_runs = self.max_runs))]
    pub fn run(mut self) -> Result<RunOutput> {
        let start = Instant::now();
        let mut local_failures = Vec::new();
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
            // Check shutdown signal
            if self.shutdown_signal.load(Ordering::Relaxed) {
                break;
            }

            // Check timeout
            let should_break = match self.timeout {
                Some(t) => start.elapsed() > t,
                None => false,
            };
            if should_break {
                break;
            }

            // Try snapshot and print progress
            if let Some(snapshot) = self.shared_metrics.try_snapshot() {
                info!(
                    elapsed = ?snapshot.elapsed,
                    runs = snapshot.runs,
                    calls = snapshot.calls,
                    gas = snapshot.gas,
                    "fuzz",
                );
            }

            // Get the next corpus item
            let item = self.shared_corpus.next_item(&mut self.rng);

            // Convert corpus item to transactions
            let transactions: Vec<crate::evm::chain::Transaction> = item
                .calls
                .iter()
                .chain(invariant_calls.iter())
                .map(|call| call.into_transaction(self.target_address))
                .collect();
            let calls_count = transactions.len();

            // Execute transactions
            let exec = self.chain.exec(&transactions)?;

            // Update shared coverage and shared corpus
            let coverage = exec.coverage.context("coverage expected")?;
            let coverage_update = self.shared_coverage.merge(&coverage);
            if SharedCoverage::is_interesting(&coverage_update) {
                self.shared_corpus
                    .add_item(item)
                    .context("failed to add corpus item")?;
            }

            let gas_sum = exec.results.iter().map(|r| r.gas_used).sum::<u64>();

            total_calls += calls_count as u64;
            total_gas += gas_sum;
            self.shared_metrics.record(calls_count as u64, gas_sum);
            runs += 1;

            // Check for failed assertions
            if !exec.panic_transactions.is_empty() {
                self.shutdown_signal.store(true, Ordering::Relaxed);
                local_failures.push(FailedAssertion { transactions });
            }
        }

        info!(runs, "fuzzer run finished");
        Ok(RunOutput {
            runs,
            failures: local_failures,
            total_calls,
            total_gas,
        })
    }
}
