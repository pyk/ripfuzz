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
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::chain_id;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface ChainIdTarget {
            function getChainId() external view returns (uint256);
            function getStoredChainId() external view returns (uint256);
            function callChainIdSameValueTwice() external returns (uint256 first, uint256 second);
            function callChainIdSequence() external returns (uint256 first, uint256 second, uint256 third);
            function setup() external;
            function actionChainId() external;
            function invariant_chain_id() external view;
        }
    }

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/ChainIdTarget.sol:ChainIdTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(ChainIdTarget::setupCall::new(()).abi_encode());
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

    /// vm.chainId must set the EVM context chain_id and persist it in state.
    #[test]
    fn chain_id_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let value = U256::from(42);
        let outcome = chain_id::handle(&mut ctx, &mut state, value);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(ctx.cfg.chain_id, 42, "ctx chain_id must be updated");
        assert_eq!(
            state.block.chain_id,
            Some(value),
            "state must record the value"
        );
    }

    /// vm.chainId with a value larger than u64::MAX must saturate to u64::MAX.
    #[test]
    fn chain_id_overflow_saturates_to_u64_max() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let value = U256::from(u64::MAX) + U256::from(1);
        let outcome = chain_id::handle(&mut ctx, &mut state, value);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(ctx.cfg.chain_id, u64::MAX, "must saturate to u64::MAX");
        assert_eq!(
            state.block.chain_id,
            Some(value),
            "state must store the original U256"
        );
    }

    /// The chain id stored in contract storage during setup must match.
    #[test]
    fn chain_id_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 =
            call_uint256_getter!(&mut chain, target, ChainIdTarget::getStoredChainIdCall);
        assert_eq!(
            decoded,
            U256::from(42),
            "stored chain id must match the value set in setup"
        );
    }

    /// vm.chainId with the same value twice in one tx must yield the same
    /// block.chainid reading, proving determinism.
    #[test]
    fn chain_id_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ChainIdTarget::callChainIdSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callChainIdSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret =
            ChainIdTarget::callChainIdSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same chainId value must give identical block.chainid readings"
        );
        assert_eq!(ret.first, U256::from(42));
    }

    /// vm.chainId with different values interleaved must produce distinct
    /// block.chainid readings, proving the cheatcode is stateful.
    #[test]
    fn chain_id_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ChainIdTarget::callChainIdSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callChainIdSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = ChainIdTarget::callChainIdSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, U256::from(1), "first vm.chainId(1) must read 1");
        assert_eq!(
            ret.second,
            U256::from(42),
            "second vm.chainId(42) must read 42"
        );
        assert_eq!(ret.third, U256::from(1), "third vm.chainId(1) must read 1");
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = ChainIdTarget::invariant_chain_idCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    /// This proves vm.chainId stays consistent across multiple transactions
    /// and that invariants correctly observe the mutated state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate chainId via a sequence that ends on a different value.
        let calldata = ChainIdTarget::callChainIdSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callChainIdSequence must succeed");

        // Restore the expected chainId with an action.
        let calldata = ChainIdTarget::actionChainIdCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionChainId must succeed");

        // Invariant must pass after the action restored state.
        let calldata = ChainIdTarget::invariant_chain_idCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.chainId(42) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn chain_id_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = ChainIdTarget::actionChainIdCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionChainId must succeed");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, ChainIdTarget::getStoredChainIdCall);
        assert_eq!(
            stored,
            U256::from(42),
            "stored chain id must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionChainId must succeed on second call");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, ChainIdTarget::getStoredChainIdCall);
        assert_eq!(
            stored,
            U256::from(42),
            "stored chain id must still match after second action"
        );
    }
}
