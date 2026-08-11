//! Per-thread shrinker that minimizes a failing corpus item.
//!
//! [`InvariantShrinker`] draws a mutated copy of the current smallest failing
//! item, executes it on a fresh chain clone, and replaces the shared item if
//! the mutated sequence is still failing and strictly smaller.
//!
//! [`InvariantShrinker`] is configured via [`InvariantShrinkerConfig`] and
//! runs directly on a cloned chain.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use alloy_primitives::Address;
use anyhow::Result;

use crate::corpus::{CorpusConfig, Item, SharedFailedCorpusItem};
use crate::evm;
use crate::evm::{ExecOutput, Transaction};
use crate::fuzzers::SharedMetrics;
use crate::shrinkers::InvariantShrinkerOutput;
use crate::shrinkers::engine::{EngineConfig, ShrinkStrategy, Shrinker};

/// Per-shrinker configuration for invariant mode, configured via a fluent
/// builder API.
#[derive(Clone, Debug)]
pub struct InvariantShrinkerConfig {
    pub seed: u64,
    pub chain: evm::Chain,
    pub target_address: Address,
    pub shared_failed_corpus: SharedFailedCorpusItem,
    pub shutdown_signal: Arc<AtomicBool>,
    pub max_runs: u64,
    pub timeout: Option<Duration>,
    pub shared_metrics: SharedMetrics,
}

impl InvariantShrinkerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            seed: 0,
            chain: evm::Chain::default(),
            target_address: Address::ZERO,
            shared_failed_corpus: SharedFailedCorpusItem::new(
                Item::from(vec![]),
                CorpusConfig::new(""),
            ),
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            max_runs: 0,
            timeout: None,
            shared_metrics: SharedMetrics::new(Vec::new()),
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

    /// Set the shared failed corpus item.
    pub fn shared_failed_item(mut self, value: SharedFailedCorpusItem) -> Self {
        self.shared_failed_corpus = value;
        self
    }

    /// Set the shared shutdown signal.
    pub fn shutdown_signal(mut self, value: Arc<AtomicBool>) -> Self {
        self.shutdown_signal = value;
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
}

impl Default for InvariantShrinkerConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-thread shrinker that executes mutated call sequences and keeps the
/// smallest item that still triggers a failed assertion.
///
/// Created via [`InvariantShrinkerConfig`] and run via
/// [`InvariantShrinker::run`].
#[derive(Debug)]
pub struct InvariantShrinker(Shrinker<InvariantStrategy>);

impl InvariantShrinker {
    /// Create a new invariant shrinker with the given config.
    pub fn new(config: InvariantShrinkerConfig) -> Self {
        let InvariantShrinkerConfig {
            seed,
            chain,
            target_address,
            shared_failed_corpus,
            shutdown_signal,
            max_runs,
            timeout,
            shared_metrics,
        } = config;
        let strategy = InvariantStrategy {
            shared_failed_corpus,
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
    /// Each iteration draws a mutated copy of the current smallest failing
    /// item, executes it on a fresh chain clone, and replaces the shared item
    /// if the mutated sequence is still failing and strictly smaller.
    pub fn run(self) -> Result<InvariantShrinkerOutput> {
        self.0.run()
    }
}

/// Invariant-mode strategy: keep the smallest candidate that still triggers a
/// failed assertion.
#[derive(Debug)]
struct InvariantStrategy {
    shared_failed_corpus: SharedFailedCorpusItem,
}

impl ShrinkStrategy for InvariantStrategy {
    type Output = InvariantShrinkerOutput;

    fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        self.shared_failed_corpus.next_item(rng)
    }

    fn sequence(&self, item: &Item, target: Address) -> Vec<Transaction> {
        item.calls
            .iter()
            .map(|call| call.into_transaction(target))
            .collect()
    }

    fn observe(&self, item: Item, exec: &ExecOutput) -> Result<()> {
        if !exec.panic_transactions.is_empty() {
            self.shared_failed_corpus.replace_item(item);
        }
        Ok(())
    }

    fn output(self, runs: u64, total_calls: u64, total_gas: u64) -> InvariantShrinkerOutput {
        InvariantShrinkerOutput {
            runs,
            total_calls,
            total_gas,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;
    use alloy_primitives::Address;

    use crate::corpus::{Call, CorpusConfig, Item, SharedFailedCorpusItem};
    use crate::evm::Contract;
    use crate::evm::{Chain, ChainConfig, DEFAULT_DEPLOYER, DeployInput, SetupInput, Transaction};
    use crate::foundry::{ArtifactId, Project};
    use crate::fuzzers::SharedMetrics;
    use crate::shrinkers::{InvariantShrinker, InvariantShrinkerConfig};

    fn load_contract(id: &str) -> Contract {
        let project = Project::new("fixtures/harness-contract-with-invariants");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup(contract: &Contract) -> (Chain, Address) {
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        if let Some(ref _setup) = contract.setup_function {
            let setup_output = chain.setup(SetupInput::new(target)).unwrap();
            assert!(setup_output.result.success, "setup must succeed");
        }

        (chain, target)
    }

    fn make_call(func: Function, caller: Address) -> Call {
        Call {
            function: func,
            args: DynSolValue::Tuple(vec![]),
            value: None,
            caller,
        }
    }

    fn build_transactions(item: &Item, target: Address) -> Vec<Transaction> {
        item.calls
            .iter()
            .map(|call| call.into_transaction(target))
            .collect()
    }

    fn run_shrinker(
        chain: Chain,
        target: Address,
        all_functions: Vec<alloy_json_abi::Function>,
        signatures: Vec<String>,
        item: Item,
        max_runs: u64,
    ) -> Item {
        let corpus_config = CorpusConfig::new("").handler_functions(all_functions);
        let shared_failed_item = SharedFailedCorpusItem::new(item, corpus_config);

        let shrinker_config = InvariantShrinkerConfig::new()
            .chain(chain)
            .target_address(target)
            .shared_failed_item(shared_failed_item.clone())
            .shutdown_signal(Arc::new(AtomicBool::new(false)))
            .max_runs(max_runs)
            .seed(42)
            .shared_metrics(SharedMetrics::new(signatures));

        let shrinker = InvariantShrinker::new(shrinker_config);
        shrinker.run().unwrap();

        shared_failed_item.item()
    }

    #[test]
    fn function_level_invariant_shrinks_to_minimal_sequence() {
        let contract = load_contract("src/FunctionLevelInvariant.sol:FunctionLevelInvariant");
        let (chain, target) = deploy_and_setup(&contract);

        let one = contract
            .handler_functions
            .iter()
            .find(|f| f.name == "one")
            .unwrap()
            .clone();
        let two = contract
            .handler_functions
            .iter()
            .find(|f| f.name == "two")
            .unwrap()
            .clone();
        let three = contract
            .handler_functions
            .iter()
            .find(|f| f.name == "three")
            .unwrap()
            .clone();

        let caller = DEFAULT_DEPLOYER;

        let invariants: Vec<Call> = contract
            .invariant_functions
            .iter()
            .map(|f| make_call(f.clone(), caller))
            .collect();

        // Combine handler calls with invariants into one item for the shrinker.
        let mut calls = vec![
            make_call(one, caller),
            make_call(two.clone(), caller),
            make_call(two, caller),
            make_call(three, caller),
        ];
        calls.extend(invariants.clone());
        let item = Item::from(calls);

        // Verify the longer sequence fails.
        let txs = build_transactions(&item, target);
        let mut exec_chain = chain.clone();
        let exec = exec_chain.exec(&txs).unwrap();
        assert!(
            !exec.panic_transactions.is_empty(),
            "initial sequence must trigger assertion"
        );

        let all_functions: Vec<alloy_json_abi::Function> = contract
            .handler_functions
            .iter()
            .chain(contract.invariant_functions.iter())
            .cloned()
            .collect();
        let signatures: Vec<String> = all_functions.iter().map(|f| f.signature()).collect();

        let shrunk = run_shrinker(chain.clone(), target, all_functions, signatures, item, 5000);

        assert_eq!(
            shrunk.calls.len(),
            3,
            "shrunk sequence must be exactly 3 calls"
        );

        // Verify the shrunk sequence still fails.
        let shrunk_txs = build_transactions(&shrunk, target);
        let mut verify_chain = chain.clone();
        let verify_exec = verify_chain.exec(&shrunk_txs).unwrap();
        assert!(
            !verify_exec.panic_transactions.is_empty(),
            "shrunk sequence must still trigger assertion"
        );
    }

    #[test]
    fn system_level_invariant_shrinks_to_minimal_sequence() {
        let contract = load_contract("src/SystemLevelInvariant.sol:SystemLevelInvariant");
        let (chain, target) = deploy_and_setup(&contract);

        let advance = contract
            .handler_functions
            .iter()
            .find(|f| f.name == "advance")
            .unwrap()
            .clone();

        let caller = DEFAULT_DEPLOYER;

        let invariants: Vec<Call> = contract
            .invariant_functions
            .iter()
            .map(|f| make_call(f.clone(), caller))
            .collect();

        // Combine handler calls with invariants into one item for the shrinker.
        let mut calls = vec![
            make_call(advance.clone(), caller),
            make_call(advance.clone(), caller),
            make_call(advance.clone(), caller),
            make_call(advance, caller),
        ];
        calls.extend(invariants.clone());
        let item = Item::from(calls);

        // Verify the longer sequence fails.
        let txs = build_transactions(&item, target);
        let mut exec_chain = chain.clone();
        let exec = exec_chain.exec(&txs).unwrap();
        assert!(
            !exec.panic_transactions.is_empty(),
            "initial sequence must trigger assertion"
        );

        let all_functions: Vec<alloy_json_abi::Function> = contract
            .handler_functions
            .iter()
            .chain(contract.invariant_functions.iter())
            .cloned()
            .collect();
        let signatures: Vec<String> = all_functions.iter().map(|f| f.signature()).collect();

        let shrunk = run_shrinker(chain.clone(), target, all_functions, signatures, item, 5000);

        assert_eq!(
            shrunk.calls.len(),
            4,
            "shrunk sequence must be exactly 4 calls"
        );

        // Verify the shrunk sequence still fails.
        let shrunk_txs = build_transactions(&shrunk, target);
        let mut verify_chain = chain.clone();
        let verify_exec = verify_chain.exec(&shrunk_txs).unwrap();
        assert!(
            !verify_exec.panic_transactions.is_empty(),
            "shrunk sequence must still trigger assertion"
        );
    }
}
