//! Per-thread shrinker that minimizes a failing corpus item.
//!
//! [`Shrinker`] draws a mutated copy of the current smallest failing
//! item, executes it on a fresh chain clone, and replaces the shared item if
//! the mutated sequence is still failing and strictly smaller.
//!
//! [`Shrinker`] is configured via [`ShrinkerConfig`] and runs directly
//! on a cloned chain.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use alloy_primitives::Address;
use anyhow::Result;
use tracing::instrument;

pub use crate::shrinker::config::ShrinkerConfig;
pub use crate::shrinker::output::ShrinkerOutput;

use crate::corpus::SharedFailedCorpusItem;
use crate::evm;
use crate::evm::Transaction;
use crate::fuzzers::SharedMetrics;

mod config;
mod output;

/// Per-thread shrinker that executes mutated call sequences and keeps the
/// smallest item that still triggers a failed assertion.
///
/// Created via [`Shrinker::new`] and run via [`Shrinker::run`].
#[derive(Debug)]
pub struct Shrinker {
    chain: evm::Chain,
    target_address: Address,
    shared_failed_corpus: SharedFailedCorpusItem,
    shutdown_signal: Arc<AtomicBool>,
    max_runs: u64,
    timeout: Option<Duration>,
    shared_metrics: SharedMetrics,
    fail_on_revert: bool,
    objective_transaction: Option<Transaction>,
    rng: fastrand::Rng,
}

impl Shrinker {
    /// Create a new shrinker with the given config.
    pub fn new(config: ShrinkerConfig) -> Self {
        Self {
            chain: config.chain,
            target_address: config.target_address,
            shared_failed_corpus: config.shared_failed_corpus,
            shutdown_signal: config.shutdown_signal,
            max_runs: config.max_runs,
            timeout: config.timeout,
            shared_metrics: config.shared_metrics,
            fail_on_revert: config.fail_on_revert,
            objective_transaction: config.objective_transaction,
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

            let item = self.shared_failed_corpus.next_item(&mut self.rng);
            // checkrs: allow(clone_in_loops)
            let mut fresh_chain = self.chain.clone();
            let mut transactions: Vec<Transaction> = item
                .calls
                .iter()
                .map(|call| call.into_transaction(self.target_address))
                .collect();
            if let Some(objective_transaction) = &self.objective_transaction {
                let mut interleaved = Vec::with_capacity(transactions.len() * 2);
                for transaction in transactions {
                    interleaved.push(transaction);
                    // checkrs: allow(clone_in_loops)
                    interleaved.push(objective_transaction.clone());
                }
                transactions = interleaved;
            }
            let calls_count = transactions.len();

            let exec = fresh_chain.exec(&transactions)?;
            let gas_sum = exec.results.iter().map(|r| r.gas_used).sum::<u64>();

            total_calls += calls_count as u64;
            total_gas += gas_sum;
            self.shared_metrics.record(calls_count as u64, gas_sum);
            runs += 1;

            if exec.has_failure(self.fail_on_revert) {
                self.shared_failed_corpus.replace_item(item);
            }
        }

        Ok(ShrinkerOutput {
            runs,
            total_calls,
            total_gas,
        })
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
    use crate::shrinker::{Shrinker, ShrinkerConfig};

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

        let shrinker_config = ShrinkerConfig::new()
            .chain(chain)
            .target_address(target)
            .shared_failed_item(shared_failed_item.clone())
            .shutdown_signal(Arc::new(AtomicBool::new(false)))
            .max_runs(max_runs)
            .seed(42)
            .shared_metrics(SharedMetrics::new(signatures));

        let shrinker = Shrinker::new(shrinker_config);
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
