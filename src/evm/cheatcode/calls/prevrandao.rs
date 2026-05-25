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
    use alloy_primitives::{Address, B256, U256};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, Config, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::prevrandao;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface PrevrandaoTarget {
            function getPrevrandao() external view returns (uint256);
            function getStoredPrevrandao() external view returns (uint256);
            function callPrevrandaoSameValueTwice() external returns (uint256 first, uint256 second);
            function callPrevrandaoSequence() external returns (uint256 first, uint256 second, uint256 third);
            function callPrevrandaoAndRoll() external returns (uint256 prevrandao, uint256 number);
            function setup() external;
            function actionPrevrandao() external;
            function invariant_prevrandao() external view;
        }
    }

    const EXPECTED_PREVRANDAO: U256 = U256::from_be_bytes([
        0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
        0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
        0x42, 0x42,
    ]);

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/PrevrandaoTarget.sol:PrevrandaoTarget");
        let mut chain = Chain::empty(Config::default());
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

    /// The prevrandao stored in contract storage during setup must match.
    #[test]
    fn prevrandao_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 = call_uint256_getter!(
            &mut chain,
            target,
            PrevrandaoTarget::getStoredPrevrandaoCall
        );
        assert_eq!(
            decoded, EXPECTED_PREVRANDAO,
            "stored prevrandao must match the value set in setup"
        );
    }

    /// vm.prevrandao with the same value twice in one tx must yield the same
    /// block.prevrandao reading, proving determinism.
    #[test]
    fn prevrandao_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrevrandaoTarget::callPrevrandaoSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "callPrevrandaoSameValueTwice() must succeed"
        );
        let output = result.output.expect("must return output");
        let ret = PrevrandaoTarget::callPrevrandaoSameValueTwiceCall::abi_decode_returns(&output)
            .unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same prevrandao value must give identical block.prevrandao readings"
        );
        assert_eq!(ret.first, EXPECTED_PREVRANDAO);
    }

    /// vm.prevrandao with different values interleaved must produce distinct
    /// block.prevrandao readings, proving the cheatcode is stateful.
    #[test]
    fn prevrandao_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrevrandaoTarget::callPrevrandaoSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrevrandaoSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret =
            PrevrandaoTarget::callPrevrandaoSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first,
            U256::from(1),
            "first vm.prevrandao(1) must read 1"
        );
        assert_eq!(
            ret.second, EXPECTED_PREVRANDAO,
            "second vm.prevrandao(expected) must read expected"
        );
        assert_eq!(
            ret.third,
            U256::from(2),
            "third vm.prevrandao(2) must read 2"
        );
    }

    /// vm.prevrandao must work correctly when combined with vm.roll in the same tx.
    #[test]
    fn prevrandao_interacts_with_roll() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrevrandaoTarget::callPrevrandaoAndRollCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrevrandaoAndRoll() must succeed");
        let output = result.output.expect("must return output");
        let ret = PrevrandaoTarget::callPrevrandaoAndRollCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.prevrandao, EXPECTED_PREVRANDAO,
            "prevrandao must match expected value"
        );
        assert_eq!(
            ret.number,
            U256::from(12345u64),
            "block number must match rolled value"
        );
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = PrevrandaoTarget::invariant_prevrandaoCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    /// This proves vm.prevrandao stays consistent across multiple transactions
    /// and that invariants correctly observe the mutated state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate prevrandao via a sequence that ends on a different value.
        let calldata = PrevrandaoTarget::callPrevrandaoSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callPrevrandaoSequence must succeed");

        // Restore the expected prevrandao with an action.
        let calldata = PrevrandaoTarget::actionPrevrandaoCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionPrevrandao must succeed");

        // Invariant must pass after the action restored state.
        let calldata = PrevrandaoTarget::invariant_prevrandaoCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.prevrandao(expected) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn prevrandao_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = PrevrandaoTarget::actionPrevrandaoCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionPrevrandao must succeed");
        let stored: U256 = call_uint256_getter!(
            &mut chain,
            target,
            PrevrandaoTarget::getStoredPrevrandaoCall
        );
        assert_eq!(
            stored, EXPECTED_PREVRANDAO,
            "stored prevrandao must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            result.success,
            "actionPrevrandao must succeed on second call"
        );
        let stored: U256 = call_uint256_getter!(
            &mut chain,
            target,
            PrevrandaoTarget::getStoredPrevrandaoCall
        );
        assert_eq!(
            stored, EXPECTED_PREVRANDAO,
            "stored prevrandao must still match after second action"
        );
    }
}
