//! Replay corpus items against the chain to seed a shared coverage map.

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};
use tracing::debug;

use crate::corpus::{Call, SharedCorpus};
use crate::evm;
use crate::evm::SharedCoverage;

/// Replays all corpus items against a cloned chain to populate a shared
/// coverage map before the fuzzing campaign starts.
///
/// Configured via a fluent builder API. All fields are owned because the
/// underlying types (`SharedCoverage`, `SharedCorpus`, `Chain`) are cheap to
/// clone (`Arc`-based or `Clone`).
#[derive(Clone, Debug)]
pub struct CorpusReplayer {
    shared_coverage: SharedCoverage,
    shared_corpus: Option<SharedCorpus>,
    chain: Option<evm::Chain>,
    deployed_address: Option<Address>,
    invariant_functions: Vec<Function>,
    caller: Address,
}

impl CorpusReplayer {
    /// Create a new replayer with the shared coverage map.
    pub fn new(shared_coverage: SharedCoverage) -> Self {
        Self {
            shared_coverage,
            shared_corpus: None,
            chain: None,
            deployed_address: None,
            invariant_functions: Vec::new(),
            caller: crate::evm::DEFAULT_DEPLOYER,
        }
    }

    /// Set the shared corpus.
    pub fn shared_corpus(mut self, value: SharedCorpus) -> Self {
        self.shared_corpus = Some(value);
        self
    }

    /// Set the chain snapshot.
    pub fn chain(mut self, value: evm::Chain) -> Self {
        self.chain = Some(value);
        self
    }

    /// Set the deployed contract address.
    pub fn deployed_address(mut self, value: Address) -> Self {
        self.deployed_address = Some(value);
        self
    }

    /// Set the invariant functions to append after each corpus sequence.
    pub fn invariant_functions(mut self, value: Vec<Function>) -> Self {
        self.invariant_functions = value;
        self
    }

    /// Set the caller address used for invariant calls.
    pub fn caller(mut self, value: Address) -> Self {
        self.caller = value;
        self
    }

    /// Replay every corpus item and merge the resulting coverage into the
    /// shared coverage map.
    ///
    /// Each item is replayed on an independent chain snapshot so that the
    /// replay captures the coverage the item produced on a fresh state.
    /// If we replayed sequentially on a single chain, later items would
    /// run against mutated state and might hit code paths that were already
    /// covered by earlier items, making them appear redundant when they are
    /// not.
    pub fn replay(self) -> Result<()> {
        let shared_corpus = self.shared_corpus.context("shared_corpus is required")?;
        let chain = self.chain.context("chain is required")?;
        let deployed_address = self
            .deployed_address
            .context("deployed_address is required")?;

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

        let items = shared_corpus.items();
        debug!(count = items.len(), "replaying corpus items");

        for (idx, item) in items.iter().enumerate() {
            let transactions: Vec<evm::Transaction> = item
                .calls
                .iter()
                .chain(invariant_calls.iter())
                .map(|call| call.into_transaction(deployed_address))
                .collect();
            // checkrs: allow(clone_in_loops)
            let mut fresh_chain = chain.clone();
            let exec = fresh_chain.exec(&transactions)?;
            let coverage = exec.coverage.context("coverage is required")?;
            let update = self.shared_coverage.merge(&coverage);
            debug!(
                idx = idx + 1,
                total = items.len(),
                item_id = %item.id(),
                new_edges = update.new_edges,
                new_features = update.new_features,
                new_depths = update.new_depths,
                new_reverts = update.new_reverts,
                new_jump_edges = update.new_jump_edges,
                new_jump_features = update.new_jump_features,
                hit_count = self.shared_coverage.hit_count(),
                "corpus item replayed"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use alloy_primitives::{Address, Bytes};
    use alloy_sol_types::SolCall;

    use crate::corpus;
    use crate::corpus::replayer::CorpusReplayer;
    use crate::corpus::{Call, Item, SharedCorpus};
    use crate::evm::Contract;
    use crate::evm::SharedCoverage;
    use crate::evm::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::foundry;

    alloy_sol_types::sol! {
        interface CoverageBranch {
            function branch(bool take) external;
        }
    }

    fn load_coverage_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup(contract: &Contract) -> (Chain, Address) {
        let config = ChainConfig::default().coverage(true);
        let mut chain = Chain::new(config).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        if let Some(setup) = &contract.setup_function {
            let setup_data = Bytes::from(setup.selector().as_slice().to_vec());
            let setup_opts = SetupInput::new(target).calldata(setup_data);
            let setup = chain.setup(setup_opts).unwrap();
            assert!(setup.result.success, "setup must succeed");
        }

        (chain, target)
    }

    #[test]
    fn corpus_replayer_populates_shared_map() {
        let contract = load_coverage_fixture("src/CoverageBranch.sol:CoverageBranch");
        let (mut chain, target) = deploy_and_setup(&contract);

        // Build a single corpus item that calls branch(true).
        let func = alloy_json_abi::Function::parse(CoverageBranch::branchCall::SIGNATURE).unwrap();
        let call = Call {
            function: func,
            args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Bool(true)]),
            ..Default::default()
        };
        let item = Item::from(vec![call]);

        // Create corpus with the item.
        let corpus_dir = std::env::temp_dir().join("raptor_test_corpus");
        let _ = fs::remove_dir_all(&corpus_dir);
        let corpus_config = corpus::CorpusConfig::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(10);
        let corpus = SharedCorpus::new(corpus_config);
        corpus.add_item(item.clone()).unwrap();

        // Empty shared coverage.
        let shared_coverage = SharedCoverage::new();
        assert_eq!(shared_coverage.hit_count(), 0, "coverage should be empty");

        // Replay corpus.
        CorpusReplayer::new(shared_coverage.clone())
            .shared_corpus(corpus.clone())
            .chain(chain.clone())
            .deployed_address(target)
            .replay()
            .unwrap();

        // Coverage should now be populated.
        assert!(
            shared_coverage.hit_count() > 0,
            "shared coverage should not be empty after replay"
        );

        // Execute the same item again and merge into shared coverage.
        let exec = chain
            .exec(&vec![Transaction::new(target).calldata(
                CoverageBranch::branchCall::new((true,)).abi_encode().into(),
            )])
            .unwrap();
        let exec_coverage = exec.coverage.expect("coverage must be present");
        let update = shared_coverage.merge(&exec_coverage);

        // Since the replayer already populated the map, re-running the same
        // input should produce no new coverage.
        assert!(
            !update.is_interesting(),
            "re-running a replayed corpus item should not be interesting"
        );
    }
}
