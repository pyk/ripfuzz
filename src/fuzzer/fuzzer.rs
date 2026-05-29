//! Per-thread fuzzer that executes call sequences and reports results.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::Instant;

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};
use revm::primitives::Bytes;
use tracing::{debug, instrument};

use crate::corpus::{Call, SharedCorpus};
use crate::evm;
use crate::evm::SharedCoverage;
use crate::evm::Transaction;
use crate::fuzzer::config::Config;
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
    /// The corpus item that produced this failure.
    pub item: crate::corpus::Item,
}

impl FailedAssertion {
    /// Format this failed assertion's call sequence as a flat, Medusa-style log.
    pub fn format(&self, contract: &evm::Contract, sender: Address) -> String {
        let mut lines = Vec::new();
        for (i, tx) in self.transactions.iter().enumerate() {
            let n = i + 1;

            let block = n as u64;
            let time = n as u64;

            lines.push(format!(
                "{}) {}::{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?})",
                n,
                contract.artifact_id.name,
                format_calldata(&tx.calldata),
                block,
                time,
                u64::MAX,
                sender,
            ));
        }
        lines.join("\n")
    }
}

fn format_calldata(calldata: &Bytes) -> String {
    if calldata.is_empty() {
        return "()".into();
    }
    format!("0x{}", hex::encode(calldata))
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

            // Get the next corpus item
            let item = self.shared_corpus.next_item(&mut self.rng);

            // Create fresh chain
            // checkrs: allow(clone_in_loops)
            let mut fresh_chain = self.chain.clone();

            // Convert corpus item to transactions
            let transactions: Vec<crate::evm::Transaction> = item
                .calls
                .iter()
                .chain(invariant_calls.iter())
                .map(|call| call.into_transaction(self.target_address))
                .collect();
            let calls_count = transactions.len();

            // Execute transactions
            let exec = fresh_chain.exec(&transactions)?;

            // Update shared coverage and shared corpus
            let coverage = exec.coverage.context("coverage expected")?;
            let coverage_update = self.shared_coverage.merge(&coverage);
            let interesting = coverage_update.is_interesting();
            debug!(
                runs = runs,
                item_id = %item.id(),
                new_edges = coverage_update.new_edges,
                new_features = coverage_update.new_features,
                new_depths = coverage_update.new_depths,
                new_reverts = coverage_update.new_reverts,
                new_jump_edges = coverage_update.new_jump_edges,
                new_jump_features = coverage_update.new_jump_features,
                hit_count = self.shared_coverage.hit_count(),
                interesting,
                "coverage merge"
            );
            if interesting {
                // checkrs: allow(clone_in_loops)
                let item_to_add = item.clone();
                self.shared_corpus
                    .add_item(item_to_add)
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
                // checkrs: allow(clone_in_loops)
                let failure_item = item.clone();
                local_failures.push(FailedAssertion {
                    transactions,
                    item: failure_item,
                });
            }
        }

        Ok(RunOutput {
            runs,
            failures: local_failures,
            total_calls,
            total_gas,
        })
    }
}
