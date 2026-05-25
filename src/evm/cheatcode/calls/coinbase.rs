//! `coinbase` cheatcode - set and persist `block.coinbase`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Address,
};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    state: &mut ExecutionState,
    addr: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.beneficiary = addr;
    ctx.set_block(block);
    state.block.beneficiary = Some(addr);
    Some(outcome::success())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::coinbase;
    use crate::evm::cheatcode::state::ExecutionState;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface CoinbaseTarget {
            function getCoinbase() external view returns (address);
            function getStoredCoinbase() external view returns (address);
            function callCoinbaseSameValueTwice() external returns (address first, address second);
            function callCoinbaseSequence() external returns (address first, address second, address third);
            function setup() external;
            function actionCoinbase() external;
            function invariant_coinbase() external view;
        }
    }

    const EXPECTED_COINBASE: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/CoinbaseTarget.sol:CoinbaseTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(CoinbaseTarget::setupCall::new(()).abi_encode());
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

    /// Call a view/pure function that returns a single `address` and decode it.
    macro_rules! call_address_getter {
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

    /// vm.coinbase must set the EVM context beneficiary and persist it in state.
    #[test]
    fn coinbase_sets_context_value() {
        let mut ctx = revm::context::Context::mainnet();
        let mut state = ExecutionState::default();
        let addr = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
        let outcome = coinbase::handle(&mut ctx, &mut state, addr);
        assert!(outcome.is_some(), "must return an outcome");
        assert_eq!(
            ctx.block.beneficiary, addr,
            "ctx beneficiary must be updated"
        );
        assert_eq!(
            state.block.beneficiary,
            Some(addr),
            "state must record the value"
        );
    }

    /// The coinbase stored in contract storage during setup must match.
    #[test]
    fn coinbase_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: Address =
            call_address_getter!(&mut chain, target, CoinbaseTarget::getStoredCoinbaseCall);
        assert_eq!(
            decoded, EXPECTED_COINBASE,
            "stored coinbase must match the value set in setup"
        );
    }

    /// vm.coinbase with the same value twice in one tx must yield the same
    /// block.coinbase reading, proving determinism.
    #[test]
    fn coinbase_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = CoinbaseTarget::callCoinbaseSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callCoinbaseSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret =
            CoinbaseTarget::callCoinbaseSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same coinbase value must give identical block.coinbase readings"
        );
        assert_eq!(ret.first, EXPECTED_COINBASE);
    }

    /// vm.coinbase with different values interleaved must produce distinct
    /// block.coinbase readings, proving the cheatcode is stateful.
    #[test]
    fn coinbase_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = CoinbaseTarget::callCoinbaseSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callCoinbaseSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = CoinbaseTarget::callCoinbaseSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first,
            address!("0x1111111111111111111111111111111111111111"),
            "first vm.coinbase must read the first address"
        );
        assert_eq!(
            ret.second, EXPECTED_COINBASE,
            "second vm.coinbase must read expected coinbase"
        );
        assert_eq!(
            ret.third,
            address!("0x2222222222222222222222222222222222222222"),
            "third vm.coinbase must read the third address"
        );
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = CoinbaseTarget::invariant_coinbaseCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    /// This proves vm.coinbase stays consistent across multiple transactions
    /// and that invariants correctly observe the mutated state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate coinbase via a sequence that ends on a different value.
        let calldata = CoinbaseTarget::callCoinbaseSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callCoinbaseSequence must succeed");

        // Restore the expected coinbase with an action.
        let calldata = CoinbaseTarget::actionCoinbaseCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionCoinbase must succeed");

        // Invariant must pass after the action restored state.
        let calldata = CoinbaseTarget::invariant_coinbaseCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.coinbase(expected) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn coinbase_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = CoinbaseTarget::actionCoinbaseCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionCoinbase must succeed");
        let stored: Address =
            call_address_getter!(&mut chain, target, CoinbaseTarget::getStoredCoinbaseCall);
        assert_eq!(
            stored, EXPECTED_COINBASE,
            "stored coinbase must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionCoinbase must succeed on second call");
        let stored: Address =
            call_address_getter!(&mut chain, target, CoinbaseTarget::getStoredCoinbaseCall);
        assert_eq!(
            stored, EXPECTED_COINBASE,
            "stored coinbase must still match after second action"
        );
    }
}
