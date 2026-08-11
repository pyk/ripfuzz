//! Per-thread fuzzer for maxxing campaigns.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use anyhow::Result;
use tracing::debug;

use crate::corpus::{CorpusConfig, Item, SharedCorpus};
use crate::evm;
use crate::evm::{SharedCoverage, Transaction, TransactionResult};
use crate::fuzzers::engine::{EngineConfig, FuzzStrategy, Fuzzer};
use crate::fuzzers::{
    FailedAssertion, MaxObjective, MaxxingFuzzerCorpus, MaxxingFuzzerOutput,
    SharedFailedAssertions, SharedMetrics,
};

/// Per-fuzzer configuration for max mode, configured via a fluent builder API.
#[derive(Clone, Debug)]
pub struct MaxxingFuzzerConfig {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shared_corpus: MaxxingFuzzerCorpus,
    pub shared_coverage: SharedCoverage,
    pub shared_metrics: SharedMetrics,
    pub shared_failed_assertions: SharedFailedAssertions,
    pub shutdown_signal: Arc<AtomicBool>,
    pub caller: Address,
    pub objective: Option<MaxObjective>,
    pub max_runs: u64,
    pub gas_limit: u64,
    pub timeout: Option<Duration>,
    pub fail_on_revert: bool,
}

impl MaxxingFuzzerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_corpus: MaxxingFuzzerCorpus::new(SharedCorpus::new(CorpusConfig::new(
                PathBuf::new(),
            ))),
            shared_coverage: SharedCoverage::new(),
            shared_metrics: SharedMetrics::new(Vec::new()),
            shared_failed_assertions: SharedFailedAssertions::new(1),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            caller: evm::DEFAULT_DEPLOYER,
            objective: None,
            max_runs: 0,
            gas_limit: 12_500_000,
            timeout: None,
            fail_on_revert: false,
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
    pub fn shared_corpus(mut self, value: MaxxingFuzzerCorpus) -> Self {
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

    /// Set the shared failed assertion collection.
    pub fn shared_failed_assertions(mut self, value: SharedFailedAssertions) -> Self {
        self.shared_failed_assertions = value;
        self
    }

    /// Set whether any reverted transaction should be treated as a failed
    /// assertion.
    pub fn fail_on_revert(mut self, value: bool) -> Self {
        self.fail_on_revert = value;
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

impl Default for MaxxingFuzzerConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-thread fuzzer that executes call sequences and tracks the maximum value
/// returned by the max objective.
///
/// Created via [`MaxxingFuzzerConfig`] and run via [`MaxxingFuzzer::run`].
#[derive(Debug)]
pub struct MaxxingFuzzer(Fuzzer<MaxxingStrategy>);

impl MaxxingFuzzer {
    /// Create a new maxxing fuzzer with the given config.
    pub fn new(config: MaxxingFuzzerConfig) -> Self {
        let MaxxingFuzzerConfig {
            seed,
            chain,
            target_address,
            shared_corpus,
            shared_coverage,
            shared_metrics,
            shared_failed_assertions,
            shutdown_signal,
            caller,
            objective,
            max_runs,
            gas_limit,
            timeout,
            fail_on_revert,
        } = config;
        let strategy =
            MaxxingStrategy::new(shared_corpus, objective.unwrap_or_else(default_objective));
        Self(Fuzzer::new(
            EngineConfig {
                seed,
                chain,
                target_address,
                shared_coverage,
                shared_metrics,
                shared_failed_assertions,
                shutdown_signal,
                caller,
                max_runs,
                gas_limit,
                timeout,
                fail_on_revert,
            },
            strategy,
        ))
    }

    /// Run the fuzzer for up to `max_runs` iterations on a single thread.
    ///
    /// Each iteration executes a handler call followed by the max objective
    /// call, so a shorter prefix that first achieves a new maximum is recorded
    /// as the best sequence.
    pub fn run(self) -> Result<MaxxingFuzzerOutput> {
        self.0.run()
    }
}

/// Maxxing-mode strategy: interleave the objective call after every handler
/// call and record the highest value with its shortest prefix.
#[derive(Debug)]
struct MaxxingStrategy {
    corpus: MaxxingFuzzerCorpus,
    objective: MaxObjective,
}

impl MaxxingStrategy {
    fn new(corpus: MaxxingFuzzerCorpus, objective: MaxObjective) -> Self {
        Self { corpus, objective }
    }
}

impl FuzzStrategy for MaxxingStrategy {
    type Output = MaxxingFuzzerOutput;

    fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        self.corpus.next_item(rng)
    }

    fn sequence(
        &self,
        item: &Item,
        target: Address,
        caller: Address,
        gas_limit: u64,
    ) -> Vec<Transaction> {
        let objective_transaction = self.objective.transaction(target, caller, gas_limit);
        let stride = 2;
        let mut transactions = vec![objective_transaction; item.calls.len() * stride];
        for (i, call) in item.calls.iter().enumerate() {
            transactions[i * stride] = call.into_transaction(target);
        }
        transactions
    }

    fn observe(
        &self,
        item: &Item,
        results: &[TransactionResult],
        metrics: &SharedMetrics,
    ) -> Result<()> {
        let stride = 2;
        for (i, call) in item.calls.iter().enumerate() {
            let handler_result = &results[i * stride];
            let handler_reverts = if handler_result.success { 0 } else { 1 };
            metrics.record_function(
                &call.function.signature(),
                1,
                handler_result.gas_used,
                handler_reverts,
            );

            let result = &results[i * stride + 1];
            let reverts = if result.success { 0 } else { 1 };
            metrics.record_function(
                &self.objective.function.signature(),
                1,
                result.gas_used,
                reverts,
            );

            let (improved, improved_value) = match self.objective.decode(result) {
                Some(value) => {
                    let prefix = Item::from(item.calls[..=i].to_vec());
                    (self.corpus.record_improvement(value, prefix)?, value)
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
        Ok(())
    }

    fn note_failure(&mut self, _failure: FailedAssertion) {}

    fn add_interesting(&self, item: Item) -> Result<()> {
        self.corpus.add_coverage_item(item)
    }

    fn output(self, runs: u64, total_calls: u64, total_gas: u64) -> MaxxingFuzzerOutput {
        MaxxingFuzzerOutput {
            runs,
            total_calls,
            total_gas,
        }
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
    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;
    use alloy_primitives::{Address, U256};

    use crate::corpus::{Call, CorpusConfig, Item, SharedCorpus};
    use crate::evm::{Chain, ChainConfig, Contract, DEFAULT_DEPLOYER, DeployInput};
    use crate::foundry::{ArtifactId, Project};
    use crate::fuzzers::{MaxObjective, MaxxingFuzzerCorpus, SharedMetrics};

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

    fn uint_arg(value: U256) -> DynSolValue {
        DynSolValue::Uint(value, 256)
    }

    /// The fuzzer must record the shortest prefix that first achieves a new
    /// maximum, so a trailing call that destroys the value is not part of the
    /// best sequence.
    #[test]
    fn records_shortest_improving_prefix() {
        let contract = load_contract("src/MaxMidSequence.sol:MaxMidSequence");
        let set = handler(&contract, "set");
        let clear = handler(&contract, "clear");
        let (chain, target) = deployed(&contract);

        let objective = objective(&contract, "max_value");

        let tmp = tempfile::tempdir().unwrap();
        let config = CorpusConfig::new(tmp.path().join("corpus"))
            .handler_functions(contract.handler_functions.clone())
            .max_calls(4);
        let corpus = MaxxingFuzzerCorpus::new(SharedCorpus::new(config));
        let strategy = MaxxingStrategy::new(corpus.clone(), objective);

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
        let transactions = strategy.sequence(&item, target, DEFAULT_DEPLOYER, 12_500_000);
        let mut fresh_chain = chain.clone();
        let exec = fresh_chain.exec(&transactions).unwrap();
        let metrics = SharedMetrics::new(Vec::new());
        strategy.observe(&item, &exec.results, &metrics).unwrap();

        let best = corpus.best_item().expect("best must be recorded");
        assert_eq!(best.value, U256::from(7));
        assert_eq!(best.item.calls.len(), 1);
        assert_eq!(best.item.calls[0].function.name, "set");
    }
}
