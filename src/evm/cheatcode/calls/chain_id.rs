//! `chainId` cheatcode - set and persist `chain_id`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::U256,
};

use crate::evm::cheatcode::{inspector::CfgMut, outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv> + CfgMut>(
    ctx: &mut CTX,
    state: &mut ExecutionState,
    value: U256,
) -> Option<revm::interpreter::CallOutcome> {
    let id = u64::try_from(value).unwrap_or(u64::MAX);
    ctx.set_chain_id(id);
    state.block.chain_id = Some(value);
    Some(outcome::success())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, Config, DeployInput, ExecInput, SetupInput, Transaction};
    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface ChainIdTarget {
            function setup() external;
            function invariant_chainId() external view;
            function actionRestoreChainId() external;
            function actionMutateChainId() external;
            function actionChainIdSequence() external;
        }
    }

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/ChainIdTarget.sol:ChainIdTarget");
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// `vm.chainId(42)` used during setup must persist in both EVM config
    /// state and contract storage. The invariant checks that the stored
    /// value matches the expected canonical chain id.
    #[test]
    fn chain_id_set_in_setup_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ChainIdTarget::invariant_chainIdCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant_chainId must pass after setup"
        );
    }

    /// Re-setting the chain id in a later transaction must not corrupt the
    /// expected value. This is the core property a stateful fuzzer relies
    /// on when actions need to restore canonical network state.
    #[test]
    fn restore_chain_id_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::actionRestoreChainIdCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::invariant_chainIdCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreChainId must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring chain id"
        );
    }

    /// A single transaction can interleave multiple `vm.chainId` calls and
    /// end on the expected value without corrupting state. This proves the
    /// cheatcode is deterministic and safe to call repeatedly inside one tx.
    #[test]
    fn batch_sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::actionChainIdSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::invariant_chainIdCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionChainIdSequence must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after sequence"
        );
    }

    /// Mutating chain id and then restoring it in a sequence must leave the
    /// invariant intact. This mirrors how a stateful fuzzer would explore
    /// state mutations and then recover canonical values.
    #[test]
    fn mutate_and_restore_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::actionMutateChainIdCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::actionRestoreChainIdCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::invariant_chainIdCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "actionMutateChainId must succeed"
        );
        assert!(
            execution.results[1].success,
            "actionRestoreChainId must succeed"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass after mutate and restore"
        );
    }

    /// A cloned chain snapshot must produce the same chain id when actions
    /// are executed on the clone. This is critical for parallel fuzzing
    /// where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_chain_id() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::actionRestoreChainIdCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::invariant_chainIdCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = cloned.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreChainId must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate storage by
    /// changing chain id, and a final invariant verifies that the canonical
    /// value is still intact. This mirrors how a stateful fuzzer would use
    /// `vm.chainId` across a campaign.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::actionRestoreChainIdCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::actionMutateChainIdCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::actionRestoreChainIdCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::actionChainIdSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ChainIdTarget::invariant_chainIdCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 5);
        assert!(
            execution.results.iter().all(|r| r.success),
            "all sequence steps must succeed"
        );
    }
}
