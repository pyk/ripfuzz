//! Mutators that transform call sequences during fuzzing.

pub use abi::SequenceArgMutator;
pub use corpus::{
    SequenceHeadMutator, SequenceInterleaveMutator, SequenceSpliceMutator, SequenceTailMutator,
};
pub use sequence::{SequenceDeleteMutator, SequenceInsertMutator, SequenceSwapMutator};

use crate::fuzzer::corpus::Call;

pub mod abi;
pub mod corpus;
pub mod sequence;

/// Result of applying a mutator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationResult {
    /// The input was changed.
    Mutated,
    /// The input was left unchanged.
    Skipped,
}

/// Trait for mutators that operate on a call sequence.
pub trait Mutator: Send + Sync + std::fmt::Debug {
    /// Mutate `calls` in place.
    fn mutate(&self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult;
}

#[cfg(test)]
mod tests {
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, Config as ChainConfig, DeployInput, SetupInput};
    use crate::foundry;
    use crate::fuzzer::corpus;
    use crate::fuzzer::mutators;
    use crate::fuzzer::mutators::Mutator;
    use crate::target::Contract;

    fn load_fixture(contract_id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/challenges");
        let artifacts = project.load_artifacts().unwrap();
        let id = foundry::ArtifactId::try_from(contract_id).unwrap();
        let artifact = artifacts.get(&id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    #[test]
    fn sequence_insert_mutator_inserts_a_call() {
        let mut rng = fastrand::Rng::with_seed(42);
        let selectors: Vec<[u8; 4]> = vec![[0x12, 0x34, 0x56, 0x78]];
        let mutator = mutators::SequenceInsertMutator::new(selectors);

        let mut calls = Vec::new();
        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, mutators::MutationResult::Mutated);
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn sequence_advances_blocks_between_calls() {
        let contract = load_fixture("src/L1SimpleKnob.sol:SimpleKnob");

        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain
            .deploy(DeployInput::new(contract.initcode.clone()))
            .unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let deployed_address = deployment.address.unwrap();

        if let Some(ref setup) = contract.setup_function {
            let setup_opts = SetupInput::new(deployed_address)
                .calldata(Bytes::from(setup.selector().as_slice().to_vec()));
            let setup_output = chain.setup(setup_opts).unwrap();
            assert!(setup_output.result.success, "setup must succeed");
        }

        let one = contract
            .target_functions
            .iter()
            .find(|f| f.name == "one")
            .unwrap()
            .selector()
            .into();
        let two = contract
            .target_functions
            .iter()
            .find(|f| f.name == "two")
            .unwrap()
            .selector()
            .into();
        let three = contract
            .target_functions
            .iter()
            .find(|f| f.name == "three")
            .unwrap()
            .selector()
            .into();

        let calls = vec![
            corpus::Call {
                selector: one,
                args: vec![],
                ..Default::default()
            },
            corpus::Call {
                selector: two,
                args: vec![],
                ..Default::default()
            },
            corpus::Call {
                selector: three,
                args: vec![],
                ..Default::default()
            },
        ];

        let res = crate::fuzzer::engine::execute_sequence(
            &chain,
            &contract,
            deployed_address,
            crate::evm::chain::DEFAULT_DEPLOYER,
            &calls,
        )
        .unwrap();
        assert!(
            res.crash.is_some(),
            "invariant should be triggered (assert panic)"
        );

        for i in 1..res.call_meta.len() {
            assert!(
                res.call_meta[i].block_number > res.call_meta[i - 1].block_number,
                "call {} block ({}) should be > call {} block ({})",
                i,
                res.call_meta[i].block_number,
                i - 1,
                res.call_meta[i - 1].block_number
            );
            assert!(
                res.call_meta[i].block_timestamp >= res.call_meta[i - 1].block_timestamp,
                "call {} timestamp ({}) should be >= call {} timestamp ({})",
                i,
                res.call_meta[i].block_timestamp,
                i - 1,
                res.call_meta[i - 1].block_timestamp
            );
        }
    }
}
