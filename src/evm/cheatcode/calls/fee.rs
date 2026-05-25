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
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::fee;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface FeeTarget {
            function getBasefee() external view returns (uint256);
            function getStoredBasefee() external view returns (uint256);
            function callFeeSameValueTwice() external returns (uint256 first, uint256 second);
            function callFeeSequence() external returns (uint256 first, uint256 second, uint256 third);
            function callFeeAndWarp() external returns (uint256 basefee, uint256 timestamp);
            function setup() external;
            function actionFee() external;
            function invariant_fee() external view;
        }
    }

    const EXPECTED_BASEFEE: U256 = U256::from_limbs([42, 0, 0, 0]);

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/FeeTarget.sol:FeeTarget");
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

    /// vm.fee must set the EVM context basefee and persist it in state.
    #[test]
    fn fee_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let value = U256::from(42);
        let outcome = fee::handle(&mut ctx, &mut state, value);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(ctx.block.basefee, 42u64, "ctx basefee must be updated");
        assert_eq!(
            state.block.basefee,
            Some(value),
            "state must record the value"
        );
    }

    /// vm.fee with a value larger than u64::MAX must saturate to 0.
    #[test]
    fn fee_overflow_saturates_to_zero() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let value = U256::from(u64::MAX) + U256::from(1);
        let outcome = fee::handle(&mut ctx, &mut state, value);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(ctx.block.basefee, 0u64, "must saturate to 0");
        assert_eq!(
            state.block.basefee,
            Some(value),
            "state must store the original U256"
        );
    }

    /// The basefee stored in contract storage during setup must match.
    #[test]
    fn fee_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 =
            call_uint256_getter!(&mut chain, target, FeeTarget::getStoredBasefeeCall);
        assert_eq!(
            decoded, EXPECTED_BASEFEE,
            "stored basefee must match the value set in setup"
        );
    }

    /// vm.fee with the same value twice in one tx must yield the same
    /// block.basefee reading, proving determinism.
    #[test]
    fn fee_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = FeeTarget::callFeeSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callFeeSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = FeeTarget::callFeeSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same fee value must give identical block.basefee readings"
        );
        assert_eq!(ret.first, EXPECTED_BASEFEE);
    }

    /// vm.fee with different values interleaved must produce distinct
    /// block.basefee readings, proving the cheatcode is stateful.
    #[test]
    fn fee_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = FeeTarget::callFeeSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callFeeSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = FeeTarget::callFeeSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, U256::from(1), "first vm.fee(1) must read 1");
        assert_eq!(
            ret.second, EXPECTED_BASEFEE,
            "second vm.fee(42) must read 42"
        );
        assert_eq!(ret.third, U256::from(5), "third vm.fee(5) must read 5");
    }

    /// vm.fee must work correctly when combined with vm.warp in the same tx.
    #[test]
    fn fee_interacts_with_warp() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = FeeTarget::callFeeAndWarpCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callFeeAndWarp() must succeed");
        let output = result.output.expect("must return output");
        let ret = FeeTarget::callFeeAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.basefee, EXPECTED_BASEFEE,
            "basefee must match expected value"
        );
        assert_eq!(
            ret.timestamp,
            U256::from(1_234_567_890u64),
            "timestamp must match warped value"
        );
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = FeeTarget::invariant_feeCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate basefee via a sequence that ends on a different value.
        let calldata = FeeTarget::callFeeSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callFeeSequence must succeed");

        // Restore the expected basefee with an action.
        let calldata = FeeTarget::actionFeeCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionFee must succeed");

        // Invariant must pass after the action restored state.
        let calldata = FeeTarget::invariant_feeCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.fee(42) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn fee_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = FeeTarget::actionFeeCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionFee must succeed");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, FeeTarget::getStoredBasefeeCall);
        assert_eq!(
            stored, EXPECTED_BASEFEE,
            "stored basefee must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionFee must succeed on second call");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, FeeTarget::getStoredBasefeeCall);
        assert_eq!(
            stored, EXPECTED_BASEFEE,
            "stored basefee must still match after second action"
        );
    }
}
