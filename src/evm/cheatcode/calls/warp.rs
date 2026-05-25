//! `warp` cheatcode - set and persist `block.timestamp`.

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
    block.timestamp = value;
    ctx.set_block(block);
    state.block.timestamp = Some(value);
    Some(outcome::success())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::chain::{
        Chain, DEFAULT_DEPLOYER, DeployInput, ExecInput, SetupInput, Transaction,
    };
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::warp;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface WarpTarget {
            function getBlockTimestamp() external view returns (uint256);
            function getStoredBlockTimestamp() external view returns (uint256);
            function callWarpSameValueTwice() external returns (uint256 first, uint256 second);
            function callWarpSequence() external returns (uint256 first, uint256 second, uint256 third);
            function callWarpAndRoll() external returns (uint256 timestamp, uint256 number);
            function callWarpLargeNumber() external returns (uint256 timestamp);
            function setup() external;
            function actionWarp() external;
            function invariant_warp() external view;
        }
    }

    const EXPECTED_TIMESTAMP: U256 = U256::from_limbs([1_234_567_890, 0, 0, 0]);

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/WarpTarget.sol:WarpTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(WarpTarget::setupCall::new(()).abi_encode());
        let setup_opts = SetupInput::new(target, setup_data);
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

    /// vm.warp must set the EVM context block.timestamp and persist it in state.
    #[test]
    fn warp_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let value = U256::from(1_234_567_890u64);
        let outcome = warp::handle(&mut ctx, &mut state, value);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(
            ctx.block.timestamp, value,
            "ctx block.timestamp must be updated"
        );
        assert_eq!(
            state.block.timestamp,
            Some(value),
            "state must record the value"
        );
    }

    /// vm.warp with U256::MAX must set the EVM context block.timestamp correctly.
    #[test]
    fn warp_large_number_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let value = U256::MAX;
        let outcome = warp::handle(&mut ctx, &mut state, value);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(
            ctx.block.timestamp, value,
            "ctx block.timestamp must be updated to U256::MAX"
        );
        assert_eq!(
            state.block.timestamp,
            Some(value),
            "state must record the large value"
        );
    }

    /// The block timestamp stored in contract storage during setup must match.
    #[test]
    fn warp_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 =
            call_uint256_getter!(&mut chain, target, WarpTarget::getStoredBlockTimestampCall);
        assert_eq!(
            decoded, EXPECTED_TIMESTAMP,
            "stored block timestamp must match the value set in setup"
        );
    }

    /// vm.warp with the same value twice in one tx must yield the same
    /// block.timestamp reading, proving determinism.
    #[test]
    fn warp_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = WarpTarget::callWarpSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callWarpSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = WarpTarget::callWarpSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same warp value must give identical block.timestamp readings"
        );
        assert_eq!(ret.first, EXPECTED_TIMESTAMP);
    }

    /// vm.warp with different values interleaved must produce distinct
    /// block.timestamp readings, proving the cheatcode is stateful.
    #[test]
    fn warp_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = WarpTarget::callWarpSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callWarpSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = WarpTarget::callWarpSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, U256::from(1), "first vm.warp(1) must read 1");
        assert_eq!(
            ret.second, EXPECTED_TIMESTAMP,
            "second vm.warp(1234567890) must read 1234567890"
        );
        assert_eq!(ret.third, U256::from(5), "third vm.warp(5) must read 5");
    }

    /// vm.warp must work correctly when combined with vm.roll in the same tx.
    #[test]
    fn warp_interacts_with_roll() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = WarpTarget::callWarpAndRollCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callWarpAndRoll() must succeed");
        let output = result.output.expect("must return output");
        let ret = WarpTarget::callWarpAndRollCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.timestamp, EXPECTED_TIMESTAMP,
            "timestamp must match warped value"
        );
        assert_eq!(
            ret.number,
            U256::from(42),
            "block.number must match rolled value"
        );
    }

    /// vm.warp to U256::MAX via a contract call must succeed.
    #[test]
    fn warp_large_number_in_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = WarpTarget::callWarpLargeNumberCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callWarpLargeNumber() must succeed");
        let output = result.output.expect("must return output");
        let ret = WarpTarget::callWarpLargeNumberCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret, U256::MAX, "block.timestamp must be U256::MAX");
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = WarpTarget::invariant_warpCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Execute a sequence: mutate timestamp, restore it, then check invariant.
        let txs = vec![
            Transaction::new(
                target,
                Bytes::from(WarpTarget::callWarpSequenceCall::new(()).abi_encode()),
            ),
            Transaction::new(
                target,
                Bytes::from(WarpTarget::actionWarpCall::new(()).abi_encode()),
            ),
            Transaction::new(
                target,
                Bytes::from(WarpTarget::invariant_warpCall::new(()).abi_encode()),
            ),
        ];
        let execution = chain.exec(ExecInput::new(txs)).unwrap();
        assert_eq!(execution.results.len(), 3);
        assert!(
            execution.results[0].success,
            "callWarpSequence must succeed"
        );
        assert!(execution.results[1].success, "actionWarp must succeed");
        assert!(
            execution.results[2].success,
            "invariant must pass after action sequence"
        );
    }

    /// vm.warp(expected) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn warp_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let txs = vec![
            Transaction::new(
                target,
                Bytes::from(WarpTarget::actionWarpCall::new(()).abi_encode()),
            ),
            Transaction::new(
                target,
                Bytes::from(WarpTarget::getStoredBlockTimestampCall::new(()).abi_encode()),
            ),
            Transaction::new(
                target,
                Bytes::from(WarpTarget::actionWarpCall::new(()).abi_encode()),
            ),
            Transaction::new(
                target,
                Bytes::from(WarpTarget::getStoredBlockTimestampCall::new(()).abi_encode()),
            ),
        ];
        let execution = chain.exec(ExecInput::new(txs)).unwrap();
        assert_eq!(execution.results.len(), 4);

        assert!(execution.results[0].success, "actionWarp must succeed");
        let stored: U256 = WarpTarget::getStoredBlockTimestampCall::abi_decode_returns(
            &execution.results[1]
                .output
                .clone()
                .expect("getter must return output"),
        )
        .unwrap();
        assert_eq!(
            stored, EXPECTED_TIMESTAMP,
            "stored block timestamp must match after first action"
        );

        assert!(
            execution.results[2].success,
            "actionWarp must succeed on second call"
        );
        let stored: U256 = WarpTarget::getStoredBlockTimestampCall::abi_decode_returns(
            &execution.results[3]
                .output
                .clone()
                .expect("getter must return output"),
        )
        .unwrap();
        assert_eq!(
            stored, EXPECTED_TIMESTAMP,
            "stored block timestamp must still match after second action"
        );
    }
}
