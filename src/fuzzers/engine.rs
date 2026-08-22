//! Shared fuzzer engine: one execution loop, specialized per mode.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use alloy_primitives::{Address, B256};
use anyhow::{Context, Result};
use tracing::{debug, instrument};

use crate::corpus::Item;
use crate::evm;
use crate::evm::{SharedCoverage, Transaction, TransactionResult};
use crate::fuzzers::{
    FailedAssertion, SharedFailedAssertions, SharedMetrics, SharedStopEvent, StopEvent,
};

/// Common fuzzer configuration shared by every mode.
#[derive(Debug)]
pub(super) struct EngineConfig {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shared_coverage: SharedCoverage,
    pub shared_metrics: SharedMetrics,
    /// Failed assertion collector. `None` in modes that do not track
    /// assertions (maxxing).
    pub shared_failed_assertions: Option<SharedFailedAssertions>,
    pub shared_stop_event: SharedStopEvent,
    pub shutdown_signal: Arc<AtomicBool>,
    pub caller: Address,
    pub max_runs: u64,
    pub gas_limit: u64,
    pub timeout: Option<Duration>,
    /// Stop the campaign on the first reverted transaction instead of
    /// continuing to fuzz.
    pub stop_on_revert: bool,
}

/// What differs between fuzzing modes: input selection, sequence layout,
/// execution observation, and corpus write-back.
pub(super) trait FuzzStrategy {
    /// Result produced by one fuzzer thread.
    type Output;

    /// Draw the next input to execute.
    fn next_item(&self, rng: &mut fastrand::Rng) -> Item;

    /// Build the transaction sequence for `item`.
    fn sequence(
        &self,
        item: &Item,
        target: Address,
        caller: Address,
        gas_limit: u64,
    ) -> Vec<Transaction>;

    /// Observe the execution results: record metrics and mode-specific state.
    fn observe(
        &self,
        item: &Item,
        results: &[TransactionResult],
        metrics: &SharedMetrics,
    ) -> Result<()>;

    /// Store an item that produced new coverage.
    fn add_interesting(&self, item: Item) -> Result<()>;

    /// Assemble the thread output from run counters.
    fn output(self, runs: u64, total_calls: u64, total_gas: u64) -> Self::Output;
}

/// Generic per-thread fuzzer engine.
#[derive(Debug)]
pub(super) struct Fuzzer<S: FuzzStrategy> {
    config: EngineConfig,
    strategy: S,
}

impl<S: FuzzStrategy> Fuzzer<S> {
    /// Create a fuzzer engine from the common config and a mode strategy.
    pub(super) fn new(config: EngineConfig, strategy: S) -> Self {
        Self { config, strategy }
    }

    /// Run the fuzzer for up to `max_runs` iterations on a single thread.
    ///
    /// The loop draws an input from the corpus, executes its sequence on a
    /// fresh chain clone, merges coverage, and dispatches failures and
    /// interesting items through the strategy.
    #[instrument(skip(self), fields(max_runs = self.config.max_runs))]
    pub(super) fn run(mut self) -> Result<S::Output> {
        let start = Instant::now();
        let mut runs = 0u64;
        let mut total_calls = 0u64;
        let mut total_gas = 0u64;
        let mut rng = fastrand::Rng::with_seed(self.config.seed);

        for _ in 0..self.config.max_runs {
            // Check shutdown signal
            if self.config.shutdown_signal.load(Ordering::Relaxed) {
                break;
            }

            // Check timeout
            let should_break = match self.config.timeout {
                Some(t) => start.elapsed() > t,
                None => false,
            };
            if should_break {
                break;
            }

            // Get the next corpus item
            let item = self.strategy.next_item(&mut rng);

            // Create fresh chain
            // checkrs: allow(clone_in_loops)
            let mut fresh_chain = self.config.chain.clone();
            let transactions = self.strategy.sequence(
                &item,
                self.config.target_address,
                self.config.caller,
                self.config.gas_limit,
            );

            // Execute transactions
            let mut exec = fresh_chain.exec(&transactions)?;

            // Update shared coverage and shared corpus
            let coverage = exec.coverage.take().context("coverage expected")?;
            let failure_pc = coverage.panic_pcs.first().copied();
            let coverage_update = self.config.shared_coverage.merge(&coverage);
            let interesting = coverage_update.is_interesting();
            debug!(
                runs = runs,
                item_id = %item.id(),
                new_edges = coverage_update.new_edges,
                new_depths = coverage_update.new_depths,
                new_reverts = coverage_update.new_reverts,
                new_jump_edges = coverage_update.new_jump_edges,
                hit_count = self.config.shared_coverage.hit_count(),
                interesting,
                "coverage merge"
            );
            let has_failure = !exec.panic_transactions.is_empty();
            let failure_index = if has_failure {
                exec.results.iter().position(|r| r.is_assert_failure())
            } else {
                None
            };

            self.strategy
                .observe(&item, &exec.results, &self.config.shared_metrics)?;

            let calls_count = transactions.len() as u64;
            let gas_sum = exec.results.iter().map(|r| r.gas_used).sum::<u64>();
            self.config.shared_metrics.record(calls_count, gas_sum);
            total_calls += calls_count;
            total_gas += gas_sum;
            runs += 1;

            // Stop-on-revert: any reverted transaction halts the campaign.
            // The failing sequence is re-run with tracing afterwards so the
            // whole trace can be dumped into the log. Only the calls up to
            // and including the first reverted transaction are kept.
            if self.config.stop_on_revert && exec.results.iter().any(|r| !r.success) {
                let stop_index = exec
                    .results
                    .iter()
                    .position(|r| !r.success)
                    .context("a reverted transaction exists")?;
                self.config.shared_stop_event.set(StopEvent {
                    transactions: transactions.into_iter().take(stop_index + 1).collect(),
                });
                self.config.shutdown_signal.store(true, Ordering::Relaxed);
                break;
            }

            // Dispatch item: move into corpus / failures, cloning only when
            // both conditions are true.
            match (interesting, has_failure) {
                (true, true) => {
                    // checkrs: allow(clone_in_loops)
                    self.strategy.add_interesting(item.clone())?;
                    self.note_failure(transactions, item, failure_index, failure_pc);
                }
                (true, false) => {
                    self.strategy.add_interesting(item)?;
                }
                (false, true) => {
                    self.note_failure(transactions, item, failure_index, failure_pc);
                }
                (false, false) => {}
            }
        }

        Ok(self.strategy.output(runs, total_calls, total_gas))
    }

    /// Record an assertion failure in the shared collector and stop the
    /// campaign when the collector is full.
    fn note_failure(
        &mut self,
        transactions: Vec<Transaction>,
        item: Item,
        failure_index: Option<usize>,
        failure_pc: Option<(B256, usize)>,
    ) {
        let Some(shared_failed_assertions) = &self.config.shared_failed_assertions else {
            return;
        };
        let failure = FailedAssertion {
            transactions,
            item,
            failure_index,
            failure_pc,
        };
        if shared_failed_assertions.try_add(failure) && shared_failed_assertions.is_full() {
            self.config.shutdown_signal.store(true, Ordering::Relaxed);
        }
    }
}
