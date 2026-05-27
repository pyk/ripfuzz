//! Per-thread fuzzer that executes call sequences and reports results.

use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy_primitives::Address;
use anyhow::Result;
use tracing::{info, instrument};

use crate::evm;
use crate::fuzzer::config::Config;
use crate::fuzzer::corpus::Item;
use crate::fuzzer::corpus::Shared;
use crate::fuzzer::engine;
use crate::fuzzer::factory::{Crash, FuzzerResult};
use crate::fuzzer::metrics::SharedMetrics;
use crate::target;

/// Per-thread fuzzer that executes call sequences and reports results.
///
/// Created by [`Factory::create`](super::factory::Factory::create) and run via [`Fuzzer::run`].
pub struct Fuzzer {
    chain: evm::Chain,
    contract: Arc<target::Contract>,
    deployed_address: Address,
    config: Config,
    caller: Address,
    corpus: Shared,
    metrics: SharedMetrics,
    rng: fastrand::Rng,
}

impl Fuzzer {
    /// Create a new fuzzer with the given state.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        chain: evm::Chain,
        contract: Arc<target::Contract>,
        deployed_address: Address,
        config: Config,
        caller: Address,
        corpus: Shared,
        metrics: SharedMetrics,
        rng: fastrand::Rng,
    ) -> Self {
        Self {
            chain,
            contract,
            deployed_address,
            config,
            caller,
            corpus,
            metrics,
            rng,
        }
    }
}

impl std::fmt::Debug for Fuzzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fuzzer")
            .field("chain", &self.chain)
            .field("contract", &self.contract)
            .field("deployed_address", &self.deployed_address)
            .field("config", &self.config)
            .field("caller", &self.caller)
            .field("corpus", &self.corpus)
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl Fuzzer {
    /// Run the fuzzer for up to `max_runs` iterations on a single thread.
    ///
    /// The fuzzer loop uses the shared corpus for mutation and the shared
    /// metrics for counters. It stops early if `timeout` is reached.
    #[instrument(skip(self), fields(max_runs))]
    pub fn run(&mut self, max_runs: u64, timeout: Option<Duration>) -> Result<FuzzerResult> {
        let start = Instant::now();
        let mut local_failures = Vec::new();
        let mut runs = 0u64;
        let mut total_calls = 0u64;
        let mut total_gas = 0u64;

        for _ in 0..max_runs {
            if let Some(t) = timeout
                && start.elapsed() > t
            {
                break;
            }

            if let Some(snapshot) = self.metrics.maybe_print() {
                info!(
                    elapsed = ?snapshot.elapsed,
                    runs = snapshot.runs,
                    calls = snapshot.calls,
                    gas = snapshot.gas,
                    failures = snapshot.failures,
                    "fuzz metrics",
                );
            }

            let item = self.corpus.next_item(&mut self.rng);
            let calls = item.calls;

            let outcome = engine::execute_sequence(
                &self.chain,
                &self.contract,
                self.deployed_address,
                self.caller,
                &calls,
            )?;

            total_calls += outcome.total_calls;
            total_gas += outcome.total_gas;
            self.metrics.record(outcome.total_calls, outcome.total_gas);

            if outcome.all_ok {
                // checkrs: allow(clone_in_loops)
                let _ = self.corpus.add_item(Item::from(calls.clone()));
            }

            if let Some(crash_info) = outcome.crash {
                local_failures.push(Crash {
                    function_name: crash_info.name,
                    selector: crash_info.selector,
                    call_sequence: calls,
                });
                self.metrics.record_failure();
            }

            runs += 1;
        }

        info!(runs, "fuzzer run finished");
        Ok(FuzzerResult {
            runs,
            failures: local_failures,
            total_calls,
            total_gas,
        })
    }
}
