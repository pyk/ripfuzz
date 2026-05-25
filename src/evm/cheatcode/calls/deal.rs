//! `deal` cheatcode - set an account balance.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Address, U256},
};

use crate::evm::cheatcode::outcome;

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    value: U256,
) -> Option<revm::interpreter::CallOutcome> {
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    let mut acc = ctx
        .journal_mut()
        .load_account_mut(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    acc.data.set_balance(value);
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
    use crate::evm::cheatcode::calls::deal;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface DealTarget {
            function getBalance(address addr) external view returns (uint256);
            function callDealSameValueTwice() external returns (uint256 first, uint256 second);
            function callDealSequence() external returns (uint256 first, uint256 second, uint256 third);
            function callDealAndWarp() external returns (uint256 balance, uint256 timestamp);
            function setup() external;
            function actionDeal() external;
            function invariant_deal() external view;
        }
    }

    const DEAL_TARGET: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");

    fn expected_balance() -> U256 {
        U256::from(1000) * U256::from(1_000_000_000_000_000_000u64)
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
        let contract = load_fixture("src/DealTarget.sol:DealTarget");
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
        ($chain:expr, $target:expr, $call:ty, $args:tt) => {{
            let calldata = <$call>::new($args).abi_encode();
            let result = $chain
                .call(DEFAULT_DEPLOYER, $target, U256::ZERO, Bytes::from(calldata))
                .unwrap();
            assert!(result.success, "{} must succeed", <$call>::SIGNATURE);
            let output = result.output.expect("getter must return output");
            <$call>::abi_decode_returns(&output).unwrap()
        }};
    }

    /// vm.deal must return success without reverting.
    #[test]
    fn deal_sets_balance_without_reverting() {
        let mut ctx = revm::context::Context::mainnet();
        let addr = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
        let outcome = deal::handle(&mut ctx, addr, U256::from(42));
        assert!(outcome.is_some(), "must return an outcome");
    }

    /// The balance set during setup must be readable via the contract getter.
    #[test]
    fn deal_persists_across_transactions() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 = call_uint256_getter!(
            &mut chain,
            target,
            DealTarget::getBalanceCall,
            (DEAL_TARGET,)
        );
        assert_eq!(
            decoded,
            expected_balance(),
            "dealt balance must persist across transactions"
        );
    }

    /// vm.deal with the same value twice in one tx must yield the same
    /// balance reading, proving determinism.
    #[test]
    fn deal_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = DealTarget::callDealSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callDealSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = DealTarget::callDealSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same deal value must give identical balance readings"
        );
        assert_eq!(ret.first, expected_balance());
    }

    /// vm.deal with different values interleaved must produce distinct
    /// balance readings, proving the cheatcode is stateful.
    #[test]
    fn deal_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = DealTarget::callDealSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callDealSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = DealTarget::callDealSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first,
            U256::from(1_000_000_000_000_000_000u64),
            "first vm.deal(1 ether) must read 1 ether"
        );
        assert_eq!(
            ret.second,
            expected_balance(),
            "second vm.deal(1000 ether) must read 1000 ether"
        );
        assert_eq!(
            ret.third,
            U256::from(5_000_000_000_000_000_000u64),
            "third vm.deal(5 ether) must read 5 ether"
        );
    }

    /// vm.deal must work correctly when combined with vm.warp in the same tx.
    #[test]
    fn deal_interacts_with_warp() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = DealTarget::callDealAndWarpCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callDealAndWarp() must succeed");
        let output = result.output.expect("must return output");
        let ret = DealTarget::callDealAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.balance,
            expected_balance(),
            "balance must match dealt value"
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
        let calldata = DealTarget::invariant_dealCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate balance via a sequence that ends on a different value.
        let calldata = DealTarget::callDealSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callDealSequence must succeed");

        // Restore the expected balance with an action.
        let calldata = DealTarget::actionDealCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionDeal must succeed");

        // Invariant must pass after the action restored state.
        let calldata = DealTarget::invariant_dealCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.deal(expected) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn deal_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = DealTarget::actionDealCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionDeal must succeed");
        let stored: U256 = call_uint256_getter!(
            &mut chain,
            target,
            DealTarget::getBalanceCall,
            (DEAL_TARGET,)
        );
        assert_eq!(
            stored,
            expected_balance(),
            "stored balance must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionDeal must succeed on second call");
        let stored: U256 = call_uint256_getter!(
            &mut chain,
            target,
            DealTarget::getBalanceCall,
            (DEAL_TARGET,)
        );
        assert_eq!(
            stored,
            expected_balance(),
            "stored balance must still match after second action"
        );
    }
}
