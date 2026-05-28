//! Per-thread fuzzer that executes call sequences and reports results.

use std::time::Duration;
use std::time::Instant;

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Selector};
use anyhow::Result;
use revm::primitives::Bytes;
use tracing::{info, instrument};

use crate::evm;
use crate::evm::chain::ExecInput;
use crate::evm::coverage::SharedCoverage;
use crate::fuzzer::config::Config;
use crate::fuzzer::corpus::{Call, Item, SharedCorpus};
use crate::fuzzer::metrics::SharedMetrics;

/// Result produced by a single fuzzer thread.
#[derive(Debug, Clone)]
pub struct FuzzerResult {
    pub runs: u64,
    pub failures: Vec<Crash>,
    pub total_calls: u64,
    pub total_gas: u64,
}

/// A single crash (assert panic) discovered during fuzzing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Crash {
    pub function_name: String,
    pub selector: Selector,
    pub call_sequence: Vec<Call>,
}

/// Solidity `Panic(uint256)` selector: keccak256("Panic(uint256)")[:4]
const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];

/// Detect a Solidity `assert` failure (`Panic(0x01)`) in revert output.
fn is_assert_failure(output: &Bytes) -> bool {
    output.len() >= 36 && output[..4] == PANIC_SELECTOR && output[35] == 0x01
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
    pub fn run(mut self) -> Result<FuzzerResult> {
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
            if let Some(t) = self.timeout
                && start.elapsed() > t
            {
                break;
            }

            if let Some(snapshot) = self.shared_metrics.try_snapshot() {
                info!(
                    elapsed = ?snapshot.elapsed,
                    runs = snapshot.runs,
                    calls = snapshot.calls,
                    gas = snapshot.gas,
                    failures = snapshot.failures,
                    "fuzz",
                );
            }

            let item = self.shared_corpus.next_item(&mut self.rng);
            let calls = item.calls;

            let transactions: Vec<crate::evm::chain::Transaction> = calls
                .iter()
                .chain(invariant_calls.iter())
                .map(|call| call.into_transaction(self.target_address))
                .collect();
            let exec = self.chain.exec(ExecInput::new(transactions))?;

            let mut all_ok = true;
            let mut crash = None;
            let mut calls_count = 0u64;
            let mut gas_sum = 0u64;

            for (idx, result) in exec.results.iter().enumerate() {
                calls_count += 1;
                gas_sum += result.gas_used;

                if !result.success {
                    if let Some(ref output) = result.output
                        && is_assert_failure(output)
                    {
                        let (function_name, selector) = if idx < calls.len() {
                            (
                                // checkrs: allow(clone_in_loops)
                                calls[idx].function.name.clone(),
                                calls[idx].function.selector(),
                            )
                        } else {
                            let inv = &invariant_calls[idx - calls.len()];
                            (
                                // checkrs: allow(clone_in_loops)
                                inv.function.name.clone(),
                                inv.function.selector(),
                            )
                        };
                        crash = Some(Crash {
                            function_name,
                            selector,
                            // checkrs: allow(clone_in_loops)
                            call_sequence: calls.clone(),
                        });
                    }
                    all_ok = false;
                    break;
                }
            }

            total_calls += calls_count;
            total_gas += gas_sum;
            self.shared_metrics.record(calls_count, gas_sum);

            if let Some(coverage) = exec.coverage {
                let _ = self.shared_coverage.merge(&coverage);
            }

            if all_ok {
                // checkrs: allow(clone_in_loops)
                let _ = self.shared_corpus.add_item(
                    // checkrs: allow(clone_in_loops)
                    Item::from(calls.clone()),
                );
            }

            if let Some(crash) = crash {
                local_failures.push(crash);
                self.shared_metrics.record_failure();
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
