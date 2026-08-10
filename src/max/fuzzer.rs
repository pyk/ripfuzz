//! Per-thread fuzzer for max mode.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use anyhow::{Context, Result};
use tracing::{debug, instrument};

use crate::corpus::{CorpusConfig, Item, SharedCorpus};
use crate::evm;
use crate::evm::{SharedCoverage, Transaction};
use crate::fuzzer::SharedMetrics;
use crate::max::corpus::MaxFuzzerCorpus;
use crate::max::objective::MaxObjective;
use crate::max::output::MaxFuzzerOutput;

/// Per-fuzzer configuration for max mode, configured via a fluent builder API.
#[derive(Clone, Debug)]
pub struct MaxFuzzerConfig {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shared_corpus: MaxFuzzerCorpus,
    pub shared_coverage: SharedCoverage,
    pub shared_metrics: SharedMetrics,
    pub shutdown_signal: Arc<AtomicBool>,
    pub caller: Address,
    pub objective: Option<MaxObjective>,
    pub max_runs: u64,
    pub gas_limit: u64,
    pub timeout: Option<Duration>,
}

impl MaxFuzzerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_corpus: MaxFuzzerCorpus::new(SharedCorpus::new(CorpusConfig::new(
                PathBuf::new(),
            ))),
            shared_coverage: SharedCoverage::new(),
            shared_metrics: SharedMetrics::new(Vec::new()),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            caller: evm::DEFAULT_DEPLOYER,
            objective: None,
            max_runs: 0,
            gas_limit: 12_500_000,
            timeout: None,
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
    pub fn shared_corpus(mut self, value: MaxFuzzerCorpus) -> Self {
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

    /// Set the shared shutdown signal.
    pub fn shutdown_signal(mut self, value: Arc<AtomicBool>) -> Self {
        self.shutdown_signal = value;
        self
    }

    /// Set the caller address.
    pub fn caller(mut self, value: Address) -> Self {
        self.caller = value;
        self
    }

    /// Set the max objective to maximize.
    pub fn objective(mut self, value: MaxObjective) -> Self {
        self.objective = Some(value);
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
}

impl Default for MaxFuzzerConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-thread fuzzer that executes call sequences and tracks the maximum value
/// returned by the max objective.
///
/// Created via [`MaxFuzzerConfig`] and run via [`MaxFuzzer::run`].
#[derive(Debug)]
pub struct MaxFuzzer {
    chain: evm::Chain,
    target_address: Address,
    shared_corpus: MaxFuzzerCorpus,
    shared_coverage: SharedCoverage,
    shared_metrics: SharedMetrics,
    shutdown_signal: Arc<AtomicBool>,
    caller: Address,
    objective: MaxObjective,
    max_runs: u64,
    gas_limit: u64,
    timeout: Option<Duration>,
    rng: fastrand::Rng,
}

impl MaxFuzzer {
    /// Create a new max fuzzer with the given config.
    pub fn new(config: MaxFuzzerConfig) -> Self {
        Self {
            chain: config.chain,
            target_address: config.target_address,
            shared_corpus: config.shared_corpus,
            shared_coverage: config.shared_coverage,
            shared_metrics: config.shared_metrics,
            shutdown_signal: config.shutdown_signal,
            caller: config.caller,
            objective: config.objective.unwrap_or_else(default_objective),
            max_runs: config.max_runs,
            gas_limit: config.gas_limit,
            timeout: config.timeout,
            rng: fastrand::Rng::with_seed(config.seed),
        }
    }

    /// Run the fuzzer for up to `max_runs` iterations on a single thread.
    ///
    /// Each iteration executes a handler call followed by the max objective
    /// call, so a shorter prefix that first achieves a new maximum is recorded
    /// as the best sequence.
    #[instrument(skip(self), fields(max_runs = self.max_runs))]
    pub fn run(mut self) -> Result<MaxFuzzerOutput> {
        let start = Instant::now();
        let mut runs = 0u64;
        let mut total_calls = 0u64;
        let mut total_gas = 0u64;

        let objective_transaction =
            self.objective
                .transaction(self.target_address, self.caller, self.gas_limit);

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

            let item = self.shared_corpus.next_item(&mut self.rng);
            let (calls_count, gas_sum) = self.evaluate_item(&item, &objective_transaction)?;
            total_calls += calls_count;
            total_gas += gas_sum;
            self.shared_metrics.record(calls_count, gas_sum);
            runs += 1;
        }

        Ok(MaxFuzzerOutput {
            runs,
            total_calls,
            total_gas,
        })
    }

    /// Execute one corpus item and record the best value for the objective.
    ///
    /// Each handler call is followed by the max objective call, so a shorter
    /// prefix that first achieves a new maximum is recorded as the best
    /// sequence. Returns `(calls, gas)` for the executed transactions.
    fn evaluate_item(
        &self,
        item: &Item,
        objective_transaction: &Transaction,
    ) -> Result<(u64, u64)> {
        // checkrs: allow(clone_in_loops)
        let mut fresh_chain = self.chain.clone();

        let stride = 2;
        let mut transactions = vec![objective_transaction.clone(); item.calls.len() * stride];
        for (i, call) in item.calls.iter().enumerate() {
            transactions[i * stride] = call.into_transaction(self.target_address);
        }

        let mut exec = fresh_chain.exec(&transactions)?;
        let coverage = exec.coverage.take().context("coverage expected")?;
        let coverage_update = self.shared_coverage.merge(&coverage);

        for (i, call) in item.calls.iter().enumerate() {
            let handler_result = &exec.results[i * stride];
            let handler_reverts = if handler_result.success { 0 } else { 1 };
            self.shared_metrics.record_function(
                &call.function.signature(),
                1,
                handler_result.gas_used,
                handler_reverts,
            );

            let result = &exec.results[i * stride + 1];
            let reverts = if result.success { 0 } else { 1 };
            self.shared_metrics.record_function(
                &self.objective.function.signature(),
                1,
                result.gas_used,
                reverts,
            );

            let (improved, improved_value) = match self.objective.decode(result) {
                Some(value) => {
                    let prefix = Item::from(item.calls[..=i].to_vec());
                    (self.shared_corpus.record_improvement(value, prefix)?, value)
                }
                None => (false, U256::ZERO),
            };
            if improved {
                debug!(
                    objective = %self.objective.function.name,
                    %improved_value,
                    "max value improved"
                );
            }
        }

        if coverage_update.is_interesting() {
            self.shared_corpus.add_coverage_item(item.clone())?;
        }

        let calls_count = transactions.len() as u64;
        let gas_sum = exec.results.iter().map(|r| r.gas_used).sum::<u64>();
        Ok((calls_count, gas_sum))
    }
}

fn default_objective() -> MaxObjective {
    MaxObjective::new(Function {
        name: String::from("max_value"),
        inputs: Vec::new(),
        outputs: Vec::new(),
        state_mutability: alloy_json_abi::StateMutability::View,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;
    use alloy_primitives::{Address, U256};

    use crate::corpus::{Call, CorpusConfig, Item, SharedCorpus};
    use crate::evm::{
        Chain, ChainConfig, Contract, DEFAULT_DEPLOYER, DeployInput, SharedCoverage, Transaction,
    };
    use crate::foundry::{ArtifactId, Project};
    use crate::fuzzer::SharedMetrics;
    use crate::max::corpus::MaxFuzzerCorpus;
    use crate::max::objective::MaxObjective;

    use super::*;

    fn load_contract(id: &str) -> Contract {
        let project = Project::new("fixtures/max-mode-harness-validation");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deployed(contract: &Contract) -> (Chain, Address) {
        let mut chain =
            Chain::new(ChainConfig::new("fixtures/max-mode-harness-validation").coverage(true))
                .unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        (chain, deployment.address.unwrap())
    }

    fn handler(contract: &Contract, name: &str) -> Function {
        contract
            .handler_functions
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("handler {name} not found"))
            .clone()
    }

    fn objective(contract: &Contract, name: &str) -> MaxObjective {
        MaxObjective::new(
            contract
                .max_functions
                .iter()
                .find(|f| f.name == name)
                .unwrap_or_else(|| panic!("max function {name} not found"))
                .clone(),
        )
    }

    fn fuzzer_config(
        chain: Chain,
        target_address: Address,
        shared_corpus: MaxFuzzerCorpus,
        objective: MaxObjective,
    ) -> MaxFuzzerConfig {
        MaxFuzzerConfig::new()
            .chain(chain)
            .target_address(target_address)
            .shared_corpus(shared_corpus)
            .shared_coverage(SharedCoverage::new())
            .shared_metrics(SharedMetrics::new(Vec::new()))
            .shutdown_signal(Arc::new(AtomicBool::new(false)))
            .caller(DEFAULT_DEPLOYER)
            .objective(objective)
            .max_runs(0)
            .gas_limit(12_500_000)
    }

    fn objective_transaction(objective: &MaxObjective, target: Address) -> Transaction {
        objective.transaction(target, DEFAULT_DEPLOYER, 12_500_000)
    }

    fn uint_arg(value: U256) -> DynSolValue {
        DynSolValue::Uint(value, 256)
    }

    /// The fuzzer must record the shortest prefix that first achieves a new
    /// maximum, so a trailing call that destroys the value is not part of the
    /// best sequence.
    #[test]
    fn evaluate_item_records_shortest_improving_prefix() {
        let contract = load_contract("src/MaxMidSequence.sol:MaxMidSequence");
        let set = handler(&contract, "set");
        let clear = handler(&contract, "clear");
        let (chain, target) = deployed(&contract);

        let objective = objective(&contract, "max_value");
        let objective_tx = objective_transaction(&objective, target);

        let tmp = tempfile::tempdir().unwrap();
        let config = CorpusConfig::new(tmp.path().join("corpus"))
            .handler_functions(contract.handler_functions.clone())
            .max_calls(4);
        let corpus = MaxFuzzerCorpus::new(SharedCorpus::new(config));
        let fuzzer = MaxFuzzer::new(fuzzer_config(chain, target, corpus.clone(), objective));

        let item = Item::from(vec![
            Call {
                function: set,
                args: DynSolValue::Tuple(vec![uint_arg(U256::from(7))]),
                ..Default::default()
            },
            Call {
                function: clear,
                args: DynSolValue::Tuple(vec![]),
                ..Default::default()
            },
        ]);
        fuzzer.evaluate_item(&item, &objective_tx).unwrap();

        let best = corpus.best_item().expect("best must be recorded");
        assert_eq!(best.value, U256::from(7));
        assert_eq!(best.item.calls.len(), 1);
        assert_eq!(best.item.calls[0].function.name, "set");
    }
}
