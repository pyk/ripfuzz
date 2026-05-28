//! `prevrandao` cheatcode - set and persist `block.prevrandao`.

use revm::{context::BlockEnv, context::ContextSetters, context_interface::ContextTr};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    state: &mut ExecutionState,
    bytes: [u8; 32],
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    ctx.set_block(block);
    state.block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    Some(outcome::success())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{B256, U256};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, Config, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::prevrandao;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface PrevrandaoTarget {
            function setup() external;
            function actionRestoreCanonical() external;
            function actionMutateValue() external;
            function actionSequence() external;
            function actionReadPrevrandao() external;
            function getPrevrandao() external view returns (uint256);
            function getStoredPrevrandao() external view returns (uint256);
            function invariant_prevrandaoMatch() external view;
        }
    }

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup() -> (Chain, revm::primitives::Address) {
        let contract = load_fixture("src/PrevrandaoTarget.sol:PrevrandaoTarget");
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    // -----------------------------------------------------------------
    // Handler-level unit test
    // -----------------------------------------------------------------

    /// vm.prevrandao must set the EVM context prevrandao and persist it in state.
    #[test]
    fn prevrandao_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let value: [u8; 32] = [
            0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
            0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
            0x42, 0x42, 0x42, 0x42,
        ];
        let outcome = prevrandao::handle(&mut ctx, &mut state, value);
        assert!(outcome.is_some(), "must return an outcome");
        let expected = B256::from(value);
        assert_eq!(
            ctx.block.prevrandao,
            Some(expected),
            "ctx prevrandao must be updated"
        );
        assert_eq!(
            state.block.prevrandao,
            Some(expected),
            "state must record the value"
        );
    }

    // -----------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------

    /// `vm.prevrandao` used during setup must persist into `chain.exec` so
    /// that the invariant passes without any additional cheatcode calls.
    #[test]
    fn setup_prevrandao_persists_into_exec() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            PrevrandaoTarget::invariant_prevrandaoMatchCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant must pass after setup"
        );
    }

    /// `vm.prevrandao` set during setup must persist through deployment and
    /// setup so that a plain `block.prevrandao` read in the first exec
    /// transaction returns the canonical value.
    #[test]
    fn prevrandao_preserved_after_deployment_and_setup() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            PrevrandaoTarget::getPrevrandaoCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(execution.results[0].success, "getPrevrandao must succeed");
        let value = PrevrandaoTarget::getPrevrandaoCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        let expected = U256::from_be_bytes([
            0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
            0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
            0x42, 0x42, 0x42, 0x42,
        ]);
        assert_eq!(
            value, expected,
            "block.prevrandao must match the canonical value set during setup"
        );
    }

    /// Reading `block.prevrandao` without calling any cheatcode in the action
    /// must still see the canonical value, proving the block environment
    /// persists across the exec.
    #[test]
    fn read_prevrandao_without_cheatcode() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrevrandaoTarget::actionReadPrevrandaoCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrevrandaoTarget::invariant_prevrandaoMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionReadPrevrandao must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after reading"
        );
    }

    /// Re-setting the canonical prevrandao in a later transaction must not
    /// corrupt the expected value. This is the core property a stateful
    /// fuzzer relies on when actions need to restore canonical block state.
    #[test]
    fn restore_prevrandao_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrevrandaoTarget::actionRestoreCanonicalCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrevrandaoTarget::invariant_prevrandaoMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestoreCanonical must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring canonical value"
        );
    }

    /// Setting a non-canonical prevrandao in an action mutates the stored
    /// state, so the invariant must fail afterward. This proves the
    /// cheatcode actually changes observable block state.
    #[test]
    fn mutate_prevrandao_breaks_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrevrandaoTarget::actionMutateValueCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrevrandaoTarget::invariant_prevrandaoMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionMutateValue must succeed"
        );
        assert!(
            !execution.results[1].success,
            "invariant must fail after mutating prevrandao"
        );
    }

    /// A sequence of prevrandao calls inside a single transaction must end
    /// on the correct canonical value, proving multiple calls in one tx
    /// compose correctly and do not interfere with each other.
    #[test]
    fn sequence_returns_correct_final_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                PrevrandaoTarget::actionSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                PrevrandaoTarget::invariant_prevrandaoMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionSequence must succeed");
        assert!(
            execution.results[1].success,
            "invariant must pass after sequence"
        );
    }

    /// A cloned chain snapshot must produce the same prevrandao state when
    /// the invariant is executed on the clone. This is critical for parallel
    /// fuzzing where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_prevrandao() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            PrevrandaoTarget::invariant_prevrandaoMatchCall::new(()).abi_encode(),
        ))];

        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant must pass on cloned chain"
        );
    }
}
