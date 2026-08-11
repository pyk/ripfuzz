//! Coverage-guided, mutational stateful fuzzer for invariant campaigns.
//!
//! [`InvariantFuzzer`] owns the execution loop: calling
//! [`next_item`](crate::corpus::SharedCorpus::next_item) to obtain an input,
//! executing it against a cloned chain, and calling
//! [`add_item`](crate::corpus::SharedCorpus::add_item) to store interesting
//! sequences discovered during execution.
//!
//! [`InvariantFuzzer`] is configured via [`InvariantFuzzerConfig`] and runs
//! directly on a cloned chain.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};

use crate::corpus::{Call, CorpusConfig, Item, SharedCorpus};
use crate::evm;
use crate::evm::{SharedCoverage, Transaction, TransactionResult};
use crate::fuzzers::engine::{EngineConfig, FuzzStrategy, Fuzzer};
use crate::fuzzers::{
    InvariantFuzzerOutput, SharedFailedAssertions, SharedMetrics, SharedStopEvent,
};

/// Per-fuzzer configuration for invariant mode, configured via a fluent
/// builder API.
#[derive(Clone, Debug)]
pub struct InvariantFuzzerConfig {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shared_corpus: SharedCorpus,
    pub shared_coverage: SharedCoverage,
    pub shared_metrics: SharedMetrics,
    pub shared_failed_assertions: SharedFailedAssertions,
    pub shared_stop_event: SharedStopEvent,
    pub shutdown_signal: Arc<AtomicBool>,
    pub caller: Address,
    pub invariant_functions: Vec<Function>,
    pub max_runs: u64,
    pub gas_limit: u64,
    pub timeout: Option<Duration>,
    pub stop_on_revert: bool,
}

impl InvariantFuzzerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_corpus: SharedCorpus::new(CorpusConfig::new(PathBuf::new())),
            shared_coverage: SharedCoverage::new(),
            shared_metrics: SharedMetrics::new(Vec::new()),
            shared_failed_assertions: SharedFailedAssertions::new(1),
            shared_stop_event: SharedStopEvent::new(),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            caller: evm::DEFAULT_DEPLOYER,
            invariant_functions: Vec::new(),
            max_runs: 0,
            gas_limit: 12_500_000,
            timeout: None,
            stop_on_revert: false,
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

    /// Set the shared corpus.
    pub fn shared_corpus(mut self, value: SharedCorpus) -> Self {
        self.shared_corpus = value;
        self
    }

    /// Set the shared coverage map.
    pub fn shared_coverage(mut self, value: SharedCoverage) -> Self {
        self.shared_coverage = value;
        self
    }

    /// Set the shared metrics.
    pub fn shared_metrics(mut self, value: SharedMetrics) -> Self {
        self.shared_metrics = value;
        self
    }

    /// Set the shared failed assertion collector.
    pub fn shared_failed_assertions(mut self, value: SharedFailedAssertions) -> Self {
        self.shared_failed_assertions = value;
        self
    }

    /// Set the shared shutdown signal.
    pub fn shutdown_signal(mut self, value: Arc<AtomicBool>) -> Self {
        self.shutdown_signal = value;
        self
    }

    /// Set the invariant functions to append after each corpus sequence.
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

    /// Set the gas limit for each fuzzer-generated transaction.
    pub fn gas_limit(mut self, value: u64) -> Self {
        self.gas_limit = value;
        self
    }

    /// Set the timeout.
    pub fn timeout(mut self, value: Option<Duration>) -> Self {
        self.timeout = value;
        self
    }

    /// Set the shared stop-on-revert event holder.
    pub fn shared_stop_event(mut self, value: SharedStopEvent) -> Self {
        self.shared_stop_event = value;
        self
    }

    /// Set whether the campaign stops on the first reverted transaction.
    pub fn stop_on_revert(mut self, value: bool) -> Self {
        self.stop_on_revert = value;
        self
    }
}

impl Default for InvariantFuzzerConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-thread fuzzer that executes call sequences and reports results.
///
/// Created via [`InvariantFuzzerConfig`] and run via [`InvariantFuzzer::run`].
#[derive(Debug)]
pub struct InvariantFuzzer(Fuzzer<InvariantStrategy>);

impl InvariantFuzzer {
    /// Create a new invariant fuzzer with the given config.
    pub fn new(config: InvariantFuzzerConfig) -> Self {
        let InvariantFuzzerConfig {
            seed,
            chain,
            target_address,
            shared_corpus,
            shared_coverage,
            shared_metrics,
            shared_failed_assertions,
            shared_stop_event,
            shutdown_signal,
            caller,
            invariant_functions,
            max_runs,
            gas_limit,
            timeout,
            stop_on_revert,
        } = config;
        let strategy = InvariantStrategy::new(shared_corpus, invariant_functions, caller);
        Self(Fuzzer::new(
            EngineConfig {
                seed,
                chain,
                target_address,
                shared_coverage,
                shared_metrics,
                shared_failed_assertions: Some(shared_failed_assertions),
                shared_stop_event,
                shutdown_signal,
                caller,
                max_runs,
                gas_limit,
                timeout,
                stop_on_revert,
            },
            strategy,
        ))
    }

    /// Run the fuzzer for up to `max_runs` iterations on a single thread.
    ///
    /// The fuzzer loop uses the shared corpus for mutation and the shared
    /// metrics for counters. It stops early if `timeout` is reached.
    pub fn run(self) -> Result<InvariantFuzzerOutput> {
        self.0.run()
    }
}

/// Invariant-mode strategy: append invariant calls after each corpus sequence
/// and store coverage-interesting items in the shared corpus.
#[derive(Debug)]
struct InvariantStrategy {
    corpus: SharedCorpus,
    invariant_calls: Vec<Call>,
}

impl InvariantStrategy {
    fn new(corpus: SharedCorpus, invariant_functions: Vec<Function>, caller: Address) -> Self {
        let invariant_calls: Vec<Call> = invariant_functions
            .iter()
            // checkrs: allow(clone_in_iterator)
            .map(|func| Call {
                function: func.clone(),
                args: DynSolValue::Tuple(vec![]),
                value: None,
                caller,
            })
            .collect();
        Self {
            corpus,
            invariant_calls,
        }
    }
}

impl FuzzStrategy for InvariantStrategy {
    type Output = InvariantFuzzerOutput;

    fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        self.corpus.next_item(rng)
    }

    fn sequence(
        &self,
        item: &Item,
        target: Address,
        _caller: Address,
        gas_limit: u64,
    ) -> Vec<Transaction> {
        item.calls
            .iter()
            .chain(self.invariant_calls.iter())
            .map(|call| call.into_transaction(target))
            .map(|tx| tx.gas_limit(gas_limit))
            .collect()
    }

    fn observe(
        &self,
        item: &Item,
        results: &[TransactionResult],
        metrics: &SharedMetrics,
    ) -> Result<()> {
        for (call, result) in item
            .calls
            .iter()
            .chain(self.invariant_calls.iter())
            .zip(results.iter())
        {
            let signature = call.function.signature();
            let calls = 1;
            let gas = result.gas_used;
            let reverts = if result.success { 0 } else { 1 };
            metrics.record_function(&signature, calls, gas, reverts);
        }
        Ok(())
    }

    fn add_interesting(&self, item: Item) -> Result<()> {
        self.corpus
            .add_item(item)
            .context("failed to add corpus item")
    }

    fn output(self, runs: u64, total_calls: u64, total_gas: u64) -> InvariantFuzzerOutput {
        InvariantFuzzerOutput {
            runs,
            total_calls,
            total_gas,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;

    use crate::corpus::Item;
    use crate::evm::Contract;
    use crate::evm::Transaction;
    use crate::foundry;
    use crate::fuzzers::FailedAssertion;

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
