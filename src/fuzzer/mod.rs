//! Coverage-guided, mutational stateful fuzzer.
//!
//! [`Fuzzer`](Fuzzer) owns the execution loop: calling
//! [`next_item`](crate::corpus::SharedCorpus::next_item) to obtain an input,
//! executing it against a cloned chain, and calling
//! [`add_item`](crate::corpus::SharedCorpus::add_item) to store interesting
//! sequences discovered during execution.
//!
//! [`Fuzzer`](Fuzzer) is configured via [`Config`](config::Config)
//! and runs directly on a cloned chain.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::Instant;

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};
use tracing::{debug, instrument};

pub use crate::fuzzer::assertions::{FailedAssertion, SharedFailedAssertions};
pub use crate::fuzzer::config::FuzzerConfig;
pub use crate::fuzzer::metrics::{FunctionMetricsSnapshot, SharedMetrics, Snapshot};
pub use crate::fuzzer::output::FuzzerOutput;

use crate::corpus::{Call, SharedCorpus};
use crate::evm;
use crate::evm::SharedCoverage;

mod assertions;
mod config;
mod metrics;
mod output;

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
    shared_failed_assertions: SharedFailedAssertions,
    shutdown_signal: Arc<AtomicBool>,
    caller: Address,
    invariant_functions: Vec<Function>,
    max_runs: u64,
    gas_limit: u64,
    timeout: Option<Duration>,
    fail_on_revert: bool,
    rng: fastrand::Rng,
}

impl Fuzzer {
    /// Create a new fuzzer with the given config.
    pub fn new(config: FuzzerConfig) -> Self {
        Self {
            chain: config.chain,
            target_address: config.target_address,
            shared_corpus: config.shared_corpus,
            shared_coverage: config.shared_coverage,
            shared_metrics: config.shared_metrics,
            shared_failed_assertions: config.shared_failed_assertions,
            shutdown_signal: config.shutdown_signal,
            caller: config.caller,
            invariant_functions: config.invariant_functions,
            max_runs: config.max_runs,
            gas_limit: config.gas_limit,
            timeout: config.timeout,
            fail_on_revert: config.fail_on_revert,
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
    pub fn run(mut self) -> Result<FuzzerOutput> {
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
            let transactions: Vec<evm::Transaction> = item
                .calls
                .iter()
                .chain(invariant_calls.iter())
                .map(|call| call.into_transaction(self.target_address))
                .map(|tx| tx.gas_limit(self.gas_limit))
                .collect();
            let calls_count = transactions.len();

            // Execute transactions
            let mut exec = fresh_chain.exec(&transactions)?;

            // Update shared coverage and shared corpus
            let coverage = exec.coverage.take().context("coverage expected")?;
            let failure_pc = coverage.panic_pcs.first().copied();
            let coverage_update = self.shared_coverage.merge(&coverage);
            let interesting = coverage_update.is_interesting();
            debug!(
                runs = runs,
                item_id = %item.id(),
                new_edges = coverage_update.new_edges,
                new_depths = coverage_update.new_depths,
                new_reverts = coverage_update.new_reverts,
                new_jump_edges = coverage_update.new_jump_edges,
                hit_count = self.shared_coverage.hit_count(),
                interesting,
                "coverage merge"
            );
            let has_failure = exec.has_failure(self.fail_on_revert);
            let failure_index = if has_failure {
                exec.results.iter().position(|r| {
                    if self.fail_on_revert {
                        !r.success
                    } else {
                        r.is_assert_failure()
                    }
                })
            } else {
                None
            };

            let gas_sum = exec.results.iter().map(|r| r.gas_used).sum::<u64>();

            for (call, result) in item
                .calls
                .iter()
                .chain(invariant_calls.iter())
                .zip(exec.results.iter())
            {
                let signature = call.function.signature();
                let calls = 1;
                let gas = result.gas_used;
                let reverts = if result.success { 0 } else { 1 };
                self.shared_metrics
                    .record_function(&signature, calls, gas, reverts);
            }

            total_calls += calls_count as u64;
            total_gas += gas_sum;
            self.shared_metrics.record(calls_count as u64, gas_sum);
            runs += 1;

            // Dispatch item: move into corpus / failures, cloning only when
            // both conditions are true.
            match (interesting, has_failure) {
                (true, true) => {
                    self.shared_corpus
                        // checkrs: allow(clone_in_loops)
                        .add_item(item.clone())
                        .context("failed to add corpus item")?;
                    let failure = FailedAssertion {
                        transactions,
                        item,
                        failure_index,
                        failure_pc,
                    };
                    // checkrs: allow(clone_in_loops)
                    if self.shared_failed_assertions.try_add(failure.clone()) {
                        local_failures.push(failure);
                    }
                    if self.shared_failed_assertions.is_full() {
                        self.shutdown_signal.store(true, Ordering::Relaxed);
                    }
                }
                (true, false) => {
                    self.shared_corpus
                        .add_item(item)
                        .context("failed to add corpus item")?;
                }
                (false, true) => {
                    let failure = FailedAssertion {
                        transactions,
                        item,
                        failure_index,
                        failure_pc,
                    };
                    // checkrs: allow(clone_in_loops)
                    if self.shared_failed_assertions.try_add(failure.clone()) {
                        local_failures.push(failure);
                    }
                    if self.shared_failed_assertions.is_full() {
                        self.shutdown_signal.store(true, Ordering::Relaxed);
                    }
                }
                (false, false) => {}
            }
        }

        Ok(FuzzerOutput {
            runs,
            failures: local_failures,
            total_calls,
            total_gas,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;

    use crate::corpus::Item;
    use crate::evm::Contract;
    use crate::evm::Transaction;
    use crate::foundry;
    use crate::fuzzer::FailedAssertion;

    #[test]
    fn format_failure_uses_numbered_call_sequence() {
        let project = foundry::Project::new("fixtures/challenges");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from("src/L1SimpleKnob.sol:SimpleKnob").unwrap();
        let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

        let transactions = vec![
            Transaction::new(Address::ZERO),
            Transaction::new(Address::ZERO),
            Transaction::new(Address::ZERO),
        ];

        let failure = FailedAssertion {
            transactions,
            item: Item::from(vec![]),
            failure_index: None,
            failure_pc: None,
        };

        let output = failure.format(&contract);
        assert!(
            output.contains("1."),
            "output should use numbered call sequence:\n{}",
            output
        );
        assert!(
            output.contains("2."),
            "output should use numbered call sequence:\n{}",
            output
        );
        assert!(
            output.contains("3."),
            "output should use numbered call sequence:\n{}",
            output
        );
        assert!(
            !output.contains("block_number="),
            "output should not use old block_number label:\n{}",
            output
        );
        assert!(
            !output.contains("gas="),
            "output should not use old gas label:\n{}",
            output
        );
        assert!(
            !output.contains("sender="),
            "output should not use old sender label:\n{}",
            output
        );
    }
}
