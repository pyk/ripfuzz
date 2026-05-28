//! Replay corpus items against the chain to seed a shared coverage map.

use alloy_primitives::Address;
use anyhow::{Context, Result};
use tracing::info;

use crate::evm;
use crate::evm::chain::ExecInput;
use crate::evm::coverage::SharedCoverage;
use crate::fuzzer::corpus::SharedCorpus;

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
}

impl CorpusReplayer {
    /// Create a new replayer with the shared coverage map.
    pub fn new(shared_coverage: SharedCoverage) -> Self {
        Self {
            shared_coverage,
            shared_corpus: None,
            chain: None,
            deployed_address: None,
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

    /// Replay every corpus item and merge the resulting coverage into the
    /// shared coverage map.
    ///
    /// This ensures that coverage discovered during a previous fuzzing
    /// session is present in the global map before the new campaign starts,
    /// so the fuzzer does not redundantly mark those inputs as "interesting".
    pub fn replay(self) -> Result<()> {
        let shared_corpus = self.shared_corpus.context("shared_corpus is required")?;
        let mut chain = self.chain.context("chain is required")?;
        let deployed_address = self
            .deployed_address
            .context("deployed_address is required")?;

        let items = shared_corpus.items();
        info!(count = items.len(), "replaying corpus items");

        for (idx, item) in items.iter().enumerate() {
            let transactions: Vec<evm::chain::Transaction> = item
                .calls
                .iter()
                .map(|call| call.into_transaction(deployed_address))
                .collect();
            let exec = chain.exec(ExecInput::new(transactions))?;
            let coverage = exec.coverage.context("coverage is required")?;
            let update = self.shared_coverage.merge(&coverage);
            info!(
                idx = idx + 1,
                total = items.len(),
                new_edges = update.new_edges,
                new_jump_edges = update.new_jump_edges,
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

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, Config, DeployInput, ExecInput, SetupInput, Transaction};
    use crate::evm::coverage::SharedCoverage;
    use crate::foundry;
    use crate::fuzzer::corpus;
    use crate::fuzzer::corpus::replayer::CorpusReplayer;
    use crate::fuzzer::corpus::{Call, Item, SharedCorpus};

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
        let mut chain = Chain::new(Config::default()).unwrap();
        chain.config.coverage = true;
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
        let corpus_config = corpus::Config::new(corpus_dir)
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
            .exec(ExecInput::new(vec![Transaction::new(target).calldata(
                CoverageBranch::branchCall::new((true,)).abi_encode().into(),
            )]))
            .unwrap();
        let exec_coverage = exec.coverage.expect("coverage must be present");
        let update = shared_coverage.merge(&exec_coverage);

        // Since the replayer already populated the map, re-running the same
        // input should produce no new coverage.
        assert!(
            !SharedCoverage::is_interesting(&update),
            "re-running a replayed corpus item should not be interesting"
        );
    }
}
