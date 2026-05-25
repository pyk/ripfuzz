//! `fee` cheatcode - set and persist `block.basefee`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::U256,
};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    state: &mut ExecutionState,
    value: U256,
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.basefee = u64::try_from(value).unwrap_or(0);
    ctx.set_block(block);
    state.block.basefee = Some(value);
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
        interface FeeTarget {
            function setup() external;
            function invariant_fee() external view;
            function actionRestoreFee() external;
            function actionMutateFee() external;
            function actionFeeSequence() external;
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
        let contract = load_fixture("src/FeeTarget.sol:FeeTarget");
        let mut chain = Chain::empty(Config::default());
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// `vm.fee(42)` used during setup must persist in EVM block state.
    /// The invariant checks that the live `block.basefee` matches the
    /// expected canonical value.
    #[test]
    fn fee_set_in_setup_matches_expected() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            FeeTarget::invariant_feeCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant_fee must pass after setup"
        );
    }

    /// Re-setting the basefee in a later transaction must not corrupt the
    /// expected value. This is the core property a stateful fuzzer relies
    /// on when actions need to restore canonical block state.
    #[test]
    fn restore_fee_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::actionRestoreFeeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::invariant_feeCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreFee must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring fee"
        );
    }

    /// A single transaction can interleave multiple `vm.fee` calls and
    /// end on the expected value without corrupting state. This proves the
    /// cheatcode is deterministic and safe to call repeatedly inside one tx.
    #[test]
    fn batch_sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::actionFeeSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::invariant_feeCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionFeeSequence must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after sequence"
        );
    }

    /// Mutating basefee and then restoring it in a sequence must leave the
    /// invariant intact. This mirrors how a stateful fuzzer would explore
    /// state mutations and then recover canonical values.
    #[test]
    fn mutate_and_restore_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::actionMutateFeeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::actionRestoreFeeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::invariant_feeCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(execution.results[0].success, "actionMutateFee must succeed");
        assert!(
            execution.results[1].success,
            "actionRestoreFee must succeed"
        );
        assert!(
            execution.results[2].success,
            "invariant must pass after mutate and restore"
        );
    }

    /// A cloned chain snapshot must produce the same basefee when actions
    /// are executed on the clone. This is critical for parallel fuzzing
    /// where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_fee() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::actionRestoreFeeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::invariant_feeCall::new(()).abi_encode(),
            )),
        ];
        let input = ExecInput::new(txs);
        let execution = cloned.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreFee must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// A realistic fuzzing sequence: multiple actions mutate block state
    /// by changing basefee, and a final invariant verifies that the canonical
    /// value is still intact. This mirrors how a stateful fuzzer would use
    /// `vm.fee` across a campaign.
    #[test]
    fn deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::actionRestoreFeeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::actionMutateFeeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::actionRestoreFeeCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::actionFeeSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                FeeTarget::invariant_feeCall::new(()).abi_encode(),
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
