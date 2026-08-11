//! Shared shrinker engine: one execution loop, specialized per mode.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use alloy_primitives::Address;
use anyhow::Result;
use tracing::instrument;

use crate::corpus::Item;
use crate::evm;
use crate::evm::{ExecOutput, Transaction};
use crate::fuzzers::SharedMetrics;

/// Common shrinker configuration shared by every mode.
#[derive(Debug)]
pub(super) struct EngineConfig {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shutdown_signal: Arc<AtomicBool>,
    pub max_runs: u64,
    pub timeout: Option<Duration>,
    pub shared_metrics: SharedMetrics,
}

/// What differs between shrinking modes: candidate source, sequence layout,
/// and the accept/reject decision.
pub(super) trait ShrinkStrategy {
    /// Result produced by one shrinker thread.
    type Output;

    /// Draw a mutated candidate to try.
    fn next_item(&self, rng: &mut fastrand::Rng) -> Item;

    /// Build the transaction sequence for the candidate.
    fn sequence(&self, item: &Item, target: Address) -> Vec<Transaction>;

    /// Observe the execution and accept or reject the candidate.
    fn observe(&self, item: Item, exec: &ExecOutput) -> Result<()>;

    /// Assemble the thread output from run counters.
    fn output(self, runs: u64, total_calls: u64, total_gas: u64) -> Self::Output;
}

/// Generic per-thread shrinker engine.
#[derive(Debug)]
pub(super) struct Shrinker<S: ShrinkStrategy> {
    config: EngineConfig,
    strategy: S,
}

impl<S: ShrinkStrategy> Shrinker<S> {
    /// Create a shrinker engine from the common config and a mode strategy.
    pub(super) fn new(config: EngineConfig, strategy: S) -> Self {
        Self { config, strategy }
    }

    /// Run the shrinker for up to `max_runs` iterations on a single thread.
    ///
    /// Each iteration draws a mutated candidate, executes it on a fresh chain
    /// clone, and lets the strategy accept or reject it.
    #[instrument(skip(self), fields(max_runs = self.config.max_runs))]
    pub(super) fn run(self) -> Result<S::Output> {
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

            // Draw a mutated candidate
            let item = self.strategy.next_item(&mut rng);

            // Create fresh chain
            // checkrs: allow(clone_in_loops)
            let mut fresh_chain = self.config.chain.clone();
            let transactions = self.strategy.sequence(&item, self.config.target_address);
            let exec = fresh_chain.exec(&transactions)?;

            let calls_count = transactions.len() as u64;
            let gas_sum = exec.results.iter().map(|r| r.gas_used).sum::<u64>();
            self.config.shared_metrics.record(calls_count, gas_sum);
            total_calls += calls_count;
            total_gas += gas_sum;
            runs += 1;

            self.strategy.observe(item, &exec)?;
        }

        Ok(self.strategy.output(runs, total_calls, total_gas))
    }
}
