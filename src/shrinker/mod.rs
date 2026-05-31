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

pub use crate::shrinker::config::ShrinkerConfig;

use crate::corpus::{Call, SharedFailedCorpusItem};
use crate::evm;
use crate::evm::Transaction;
use crate::fuzzer::SharedMetrics;

mod config;

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
    pub fn new(config: ShrinkerConfig) -> Self {
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
