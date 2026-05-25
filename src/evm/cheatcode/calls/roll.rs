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

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::roll;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface RollTarget {
            function getBlockNumber() external view returns (uint256);
            function getStoredBlockNumber() external view returns (uint256);
            function callRollSameValueTwice() external returns (uint256 first, uint256 second);
            function callRollSequence() external returns (uint256 first, uint256 second, uint256 third);
            function callRollAndWarp() external returns (uint256 number, uint256 timestamp);
            function callRollLargeNumber() external returns (uint256 number);
            function setup() external;
            function actionRoll() external;
            function invariant_roll() external view;
        }
    }

    const EXPECTED_NUMBER: U256 = U256::from_limbs([42, 0, 0, 0]);

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/RollTarget.sol:RollTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_opts = SetupInput::new(target);
        let setup = chain.setup(setup_opts).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// Execute a CALL with the cheatcode inspector enabled so that `vm.*`
    /// functions invoked by the target contract are intercepted.
    fn call_with_cheatcode_inspector(
        chain: &mut Chain,
        caller: Address,
        target: Address,
        data: Bytes,
    ) -> TransactionResult {
        let inspector = cheatcode::Inspector::default();
        let tx = revm::context::TxEnv {
            caller,
            kind: revm::primitives::TxKind::Call(target),
            data,
            gas_limit: u64::MAX,
            value: U256::ZERO,
            ..Default::default()
        };
        let (result, _) = chain.inspect(tx, inspector).unwrap();
        result
    }

    /// Call a view/pure function that returns a single `uint256` and decode it.
    macro_rules! call_uint256_getter {
        ($chain:expr, $target:expr, $call:ty) => {{
            let calldata = <$call>::new(()).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

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

    /// The block number stored in contract storage during setup must match.
    #[test]
    fn roll_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 =
            call_uint256_getter!(&mut chain, target, RollTarget::getStoredBlockNumberCall);
        assert_eq!(
            decoded, EXPECTED_NUMBER,
            "stored block number must match the value set in setup"
        );
    }

    /// vm.roll with the same value twice in one tx must yield the same
    /// block.number reading, proving determinism.
    #[test]
    fn roll_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = RollTarget::callRollSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callRollSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = RollTarget::callRollSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same roll value must give identical block.number readings"
        );
        assert_eq!(ret.first, EXPECTED_NUMBER);
    }

    /// vm.roll with different values interleaved must produce distinct
    /// block.number readings, proving the cheatcode is stateful.
    #[test]
    fn roll_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = RollTarget::callRollSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callRollSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = RollTarget::callRollSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, U256::from(1), "first vm.roll(1) must read 1");
        assert_eq!(
            ret.second, EXPECTED_NUMBER,
            "second vm.roll(42) must read 42"
        );
        assert_eq!(ret.third, U256::from(5), "third vm.roll(5) must read 5");
    }

    /// vm.roll must work correctly when combined with vm.warp in the same tx.
    #[test]
    fn roll_interacts_with_warp() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = RollTarget::callRollAndWarpCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callRollAndWarp() must succeed");
        let output = result.output.expect("must return output");
        let ret = RollTarget::callRollAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.number, EXPECTED_NUMBER,
            "block.number must match rolled value"
        );
        assert_eq!(
            ret.timestamp,
            U256::from(1_234_567_890u64),
            "timestamp must match warped value"
        );
    }

    /// vm.roll to U256::MAX via a contract call must succeed.
    #[test]
    fn roll_large_number_in_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = RollTarget::callRollLargeNumberCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callRollLargeNumber() must succeed");
        let output = result.output.expect("must return output");
        let ret = RollTarget::callRollLargeNumberCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret, U256::MAX, "block.number must be U256::MAX");
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = RollTarget::invariant_rollCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate block number via a sequence that ends on a different value.
        let calldata = RollTarget::callRollSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callRollSequence must succeed");

        // Restore the expected block number with an action.
        let calldata = RollTarget::actionRollCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionRoll must succeed");

        // Invariant must pass after the action restored state.
        let calldata = RollTarget::invariant_rollCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.roll(42) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn roll_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = RollTarget::actionRollCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionRoll must succeed");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, RollTarget::getStoredBlockNumberCall);
        assert_eq!(
            stored, EXPECTED_NUMBER,
            "stored block number must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionRoll must succeed on second call");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, RollTarget::getStoredBlockNumberCall);
        assert_eq!(
            stored, EXPECTED_NUMBER,
            "stored block number must still match after second action"
        );
    }
}
