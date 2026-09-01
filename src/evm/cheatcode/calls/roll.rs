//! `roll` cheatcode - set and persist `block.number`.

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
    block.number = value;
    ctx.set_block(block);
    state.block.number = Some(value);
    Some(outcome::success())
}

#[cfg(test)]
mod tests {

    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::compilers::solc::{Solc, SolcOutput};
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::roll;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::harness::HarnessId;

    fn compile_fixture(root: &str, target: &str) -> SolcOutput {
        let id = HarnessId::try_from(target).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        Solc::new()
            .with_version("0.8.36")
            .with_root(root)
            .with_target(&id.path)
            .with_name(&id.name)
            .with_out(tmp.path().join("out"))
            .compile()
            .unwrap()
    }

    alloy_sol_types::sol! {
        interface RollHarness {
            function setup() external;
            function actionRestoreCanonical() external;
            function actionMutateValue() external;
            function actionSequence() external;
            function actionReadBlockNumber() external;
            function getBlockNumber() external view returns (uint256);
            function getStoredBlockNumber() external view returns (uint256);
            function invariant_blockNumberMatch() external view;
        }
    }

    const CANONICAL: U256 = U256::from_limbs([42, 0, 0, 0]);

    fn load_initcode(id: &str) -> String {
        compile_fixture("fixtures/evm/cheatcodes", id)
            .initcode()
            .unwrap()
            .to_owned()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let initcode = load_initcode("RollHarness.sol:RollHarness");
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    // Harness-level unit tests

    /// vm.roll must set the EVM context block.number and persist it in state.
    #[test]
    fn roll_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let value = U256::from(42);
        let outcome = roll::handle(&mut ctx, &mut state, value);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(ctx.block.number, value, "ctx block.number must be updated");
        assert_eq!(
            state.block.number,
            Some(value),
            "state must record the value"
        );
    }

    /// vm.roll with U256::MAX must set the EVM context block.number correctly.
    #[test]
    fn roll_large_number_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let value = U256::MAX;
        let outcome = roll::handle(&mut ctx, &mut state, value);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(
            ctx.block.number, value,
            "ctx block.number must be updated to U256::MAX"
        );
        assert_eq!(
            state.block.number,
            Some(value),
            "state must record the large value"
        );
    }

    // Integration tests

    /// `vm.roll` used during setup must persist into `chain.exec` so that the
    /// invariant passes without any additional cheatcode calls.
    #[test]
    fn setup_roll_persists_into_exec() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            RollHarness::invariant_blockNumberMatchCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant must pass after setup"
        );
    }

    /// `vm.roll` set during setup must persist through deployment and setup
    /// so that a plain `block.number` read in the first exec transaction
    /// returns the canonical value.
    #[test]
    fn block_number_preserved_after_deployment_and_setup() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            RollHarness::getBlockNumberCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(execution.results[0].success, "getBlockNumber must succeed");
        let value = RollHarness::getBlockNumberCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(
            value, CANONICAL,
            "block.number must match the canonical value set during setup"
        );
    }

    /// Reading `block.number` without calling any cheatcode in the action
    /// must still see the canonical value, proving the block environment
    /// persists across the exec.
    #[test]
    fn read_block_number_without_cheatcode() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                RollHarness::actionReadBlockNumberCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                RollHarness::invariant_blockNumberMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionReadBlockNumber must succeed"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after reading"
        );
    }

    /// Re-setting the canonical block number in a later transaction must not
    /// corrupt the expected value. This is the core property a stateful
    /// fuzzer relies on when actions need to restore canonical block state.
    #[test]
    fn restore_block_number_preserves_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                RollHarness::actionRestoreCanonicalCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                RollHarness::invariant_blockNumberMatchCall::new(()).abi_encode(),
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

    /// Setting a non-canonical block number in an action mutates the stored
    /// state, so the invariant must fail afterward. This proves the cheatcode
    /// actually changes observable block state.
    #[test]
    fn mutate_block_number_breaks_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                RollHarness::actionMutateValueCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                RollHarness::invariant_blockNumberMatchCall::new(()).abi_encode(),
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
            "invariant must fail after mutating block number"
        );
    }

    /// A sequence of roll calls inside a single transaction must end on the
    /// correct canonical value, proving multiple calls in one tx compose
    /// correctly and do not interfere with each other.
    #[test]
    fn sequence_returns_correct_final_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                RollHarness::actionSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                RollHarness::invariant_blockNumberMatchCall::new(()).abi_encode(),
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

    /// A cloned chain snapshot must produce the same block number when the
    /// invariant is executed on the clone. This is critical for parallel
    /// fuzzing where each worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_block_number() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            RollHarness::invariant_blockNumberMatchCall::new(()).abi_encode(),
        ))];

        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "invariant must pass on cloned chain"
        );
    }
}
