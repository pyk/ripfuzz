//! Per-thread shrinker for maxxing-mode results.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use anyhow::{Context, Result};

use crate::corpus::{CorpusConfig, Item, SharedCorpus};
use crate::evm;
use crate::evm::{ExecOutput, Transaction};
use crate::fuzzers::{MaxObjective, SharedMetrics};
use crate::shrinkers::engine::{EngineConfig, ShrinkStrategy, Shrinker};
use crate::shrinkers::{MaxxingShrinkerCorpus, MaxxingShrinkerOutput};

/// Per-shrinker configuration for maxxing mode, configured via a fluent
/// builder API.
#[derive(Clone, Debug)]
pub struct MaxxingShrinkerConfig {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shared_corpus: MaxxingShrinkerCorpus,
    pub shutdown_signal: Arc<AtomicBool>,
    pub objective: Option<MaxObjective>,
    pub max_runs: u64,
    pub timeout: Option<Duration>,
    pub shared_metrics: SharedMetrics,
    pub gas_limit: u64,
    pub caller: Address,
}

impl MaxxingShrinkerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_corpus: MaxxingShrinkerCorpus::new(
                Item::from(vec![]),
                U256::ZERO,
                CorpusConfig::new(""),
                SharedCorpus::new(CorpusConfig::new("")),
            ),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            objective: None,
            max_runs: 0,
            timeout: None,
            shared_metrics: SharedMetrics::new(Vec::new()),
            gas_limit: 12_500_000,
            caller: evm::DEFAULT_DEPLOYER,
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

    /// Set the shared shrinker corpus.
    pub fn shared_corpus(mut self, value: MaxxingShrinkerCorpus) -> Self {
        self.shared_corpus = value;
        self
    }

    /// Set the shared shutdown signal.
    pub fn shutdown_signal(mut self, value: Arc<AtomicBool>) -> Self {
        self.shutdown_signal = value;
        self
    }

    /// Set the objective whose value must be preserved while shrinking.
    pub fn objective(mut self, value: MaxObjective) -> Self {
        self.objective = Some(value);
        self
    }

    /// Set the maximum number of runs.
    pub fn max_runs(mut self, value: u64) -> Self {
        self.max_runs = value;
        self
    }

    /// Set the timeout.
    pub fn timeout(mut self, value: Option<Duration>) -> Self {
        self.timeout = value;
        self
    }

    /// Set the shared metrics.
    pub fn shared_metrics(mut self, value: SharedMetrics) -> Self {
        self.shared_metrics = value;
        self
    }

    /// Set the gas limit for each shrinker-generated transaction.
    pub fn gas_limit(mut self, value: u64) -> Self {
        self.gas_limit = value;
        self
    }

    /// Set the caller address.
    pub fn caller(mut self, value: Address) -> Self {
        self.caller = value;
        self
    }
}

impl Default for MaxxingShrinkerConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-thread shrinker that minimizes a maxxing result while preserving its
/// value.
///
/// Created via [`MaxxingShrinkerConfig`] and run via
/// [`MaxxingShrinker::run`].
#[derive(Debug)]
pub struct MaxxingShrinker(Shrinker<MaxxingStrategy>);

impl MaxxingShrinker {
    /// Create a new maxxing shrinker with the given config.
    pub fn new(config: MaxxingShrinkerConfig) -> Self {
        let MaxxingShrinkerConfig {
            seed,
            chain,
            target_address,
            shared_corpus,
            shutdown_signal,
            objective,
            max_runs,
            timeout,
            shared_metrics,
            gas_limit,
            caller,
        } = config;
        let strategy = MaxxingStrategy {
            shared_corpus,
            objective: objective.unwrap_or_else(default_objective),
            caller,
            gas_limit,
        };
        Self(Shrinker::new(
            EngineConfig {
                seed,
                chain,
                target_address,
                shutdown_signal,
                max_runs,
                timeout,
                shared_metrics,
            },
            strategy,
        ))
    }

    /// Run the shrinker for up to `max_runs` iterations on a single thread.
    ///
    /// Each iteration draws a mutated copy of the current best item, executes
    /// it followed by the max objective call, and accepts the candidate when it
    /// preserves or improves the stored value and shrinks the sequence.
    pub fn run(self) -> Result<MaxxingShrinkerOutput> {
        self.0.run()
    }
}

/// Maxxing-mode strategy: keep the smallest candidate that preserves or
/// improves the stored objective value.
#[derive(Debug)]
struct MaxxingStrategy {
    shared_corpus: MaxxingShrinkerCorpus,
    objective: MaxObjective,
    caller: Address,
    gas_limit: u64,
}

impl ShrinkStrategy for MaxxingStrategy {
    type Output = MaxxingShrinkerOutput;

    fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        self.shared_corpus.next_item(rng)
    }

    fn sequence(&self, item: &Item, target: Address) -> Vec<Transaction> {
        let mut transactions: Vec<Transaction> = item
            .calls
            .iter()
            .map(|call| call.into_transaction(target))
            .collect();
        transactions.push(
            self.objective
                .transaction(target, self.caller, self.gas_limit),
        );
        transactions
    }

    fn observe(&self, item: Item, exec: &ExecOutput) -> Result<()> {
        let raw_score = self
            .objective
            .decode(exec.results.last().context("max call result missing")?)
            .unwrap_or_default();
        self.shared_corpus.accept(item, raw_score);
        Ok(())
    }

    fn output(self, runs: u64, total_calls: u64, total_gas: u64) -> MaxxingShrinkerOutput {
        MaxxingShrinkerOutput {
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
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use alloy_dyn_abi::DynSolValue;
    use alloy_primitives::U256;

    use crate::corpus::{Call, CorpusConfig, Item, SharedCorpus};
    use crate::evm::{Chain, ChainConfig, Contract, DEFAULT_DEPLOYER, DeployInput};
    use crate::foundry::{ArtifactId, Project};
    use crate::fuzzers::{MaxObjective, SharedMetrics};
    use crate::shrinkers::{MaxxingShrinker, MaxxingShrinkerConfig, MaxxingShrinkerCorpus};

    fn load_contract(id: &str) -> Contract {
        let project = Project::new("fixtures/max-mode-harness-validation");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    #[test]
    fn maxxing_shrinker_removes_trailing_clear() {
        let contract = load_contract("src/MaxMidSequence.sol:MaxMidSequence");
        let set = contract
            .handler_functions
            .iter()
            .find(|f| f.name == "set")
            .unwrap()
            .clone();
        let clear = contract
            .handler_functions
            .iter()
            .find(|f| f.name == "clear")
            .unwrap()
            .clone();
        let max_value = contract
            .max_functions
            .iter()
            .find(|f| f.name == "max_value")
            .unwrap()
            .clone();

        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let set_call = Call {
            function: set,
            args: DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(7), 256)]),
            ..Default::default()
        };
        let clear_call = Call {
            function: clear,
            args: DynSolValue::Tuple(vec![]),
            ..Default::default()
        };
        let item = Item::from(vec![set_call, clear_call]);

        let tmp = tempfile::tempdir().unwrap();
        let corpus = SharedCorpus::new(CorpusConfig::new(tmp.path().join("corpus")));
        let config = CorpusConfig::new(tmp.path().join("corpus"))
            .handler_functions(contract.handler_functions.clone())
            .max_calls(4);
        let shrink_corpus = MaxxingShrinkerCorpus::new(item, U256::from(7), config, corpus);

        let shrinker_config = MaxxingShrinkerConfig::new()
            .chain(chain)
            .target_address(target)
            .shared_corpus(shrink_corpus.clone())
            .shutdown_signal(Arc::new(AtomicBool::new(false)))
            .objective(MaxObjective::new(max_value))
            .max_runs(3000)
            .seed(42)
            .shared_metrics(SharedMetrics::new(Vec::new()))
            .gas_limit(12_500_000)
            .caller(DEFAULT_DEPLOYER);

        let shrinker = MaxxingShrinker::new(shrinker_config);
        shrinker.run().unwrap();

        let final_item = shrink_corpus.item();
        assert!(
            final_item.best_score >= U256::from(7),
            "best_score must be preserved or improved"
        );
        assert_eq!(
            final_item.item.calls.len(),
            1,
            "shrunk sequence must be exactly one call"
        );
        assert_eq!(final_item.item.calls[0].function.name, "set");
    }
}
