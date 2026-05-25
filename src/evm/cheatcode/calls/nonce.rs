//! `setNonce` / `getNonce` cheatcodes.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Address, U256},
};

use crate::evm::cheatcode::outcome;

pub fn set_nonce<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    nonce: u64,
) -> Option<revm::interpreter::CallOutcome> {
    let current = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    if nonce < current {
        return Some(outcome::revert(&format!(
            "new nonce ({nonce}) must be >= current nonce ({current})"
        )));
    }
    let mut acc = ctx
        .journal_mut()
        .load_account_mut(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    acc.data.set_nonce(nonce);
    Some(outcome::success())
}

pub fn get_nonce<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let nonce = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    Some(outcome::success_u256(U256::from(nonce)))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployOptions, SetupOptions};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::nonce;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface NonceTarget {
            function setup() external;
            function getStoredNonce() external view returns (uint256);
            function getNonceExternal(address addr) external view returns (uint256);
            function callSetNonceSameValueTwice() external returns (uint256 first, uint256 second);
            function callSetNonceSequence() external returns (uint256 first, uint256 second, uint256 third);
            function callSetNonceAndDeal() external returns (uint256 nonce, uint256 balance);
            function callSetNonceAndRevertLowNonce() external;
            function actionSetNonce() external;
            function invariant_nonce() external view;
        }
    }

    const NONCE_TARGET: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
    const EXPECTED_NONCE: U256 = U256::from_limbs([42, 0, 0, 0]);

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
        let contract = load_fixture("src/NonceTarget.sol:NonceTarget");
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployOptions::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(NonceTarget::setupCall::new(()).abi_encode());
        let setup_opts = SetupOptions::new(target, setup_data);
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

    /// vm.setNonce must succeed and vm.getNonce must read the written value
    /// at the handler level.
    #[test]
    fn set_nonce_sets_nonce_and_get_nonce_reads_it() {
        let mut ctx = revm::context::Context::mainnet();
        let outcome = nonce::set_nonce(&mut ctx, NONCE_TARGET, 42);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = nonce::get_nonce(&mut ctx, NONCE_TARGET);
        assert!(outcome.is_some(), "must return an outcome");
        let decoded =
            U256::from_be_bytes::<32>(outcome.unwrap().result.output.as_ref().try_into().unwrap());
        assert_eq!(
            decoded,
            U256::from_limbs([42, 0, 0, 0]),
            "get_nonce must read 42"
        );
    }

    /// vm.setNonce with a value lower than the current nonce must revert.
    #[test]
    fn set_nonce_lower_than_current_reverts() {
        let mut ctx = revm::context::Context::mainnet();
        // First set nonce to 10.
        let outcome = nonce::set_nonce(&mut ctx, NONCE_TARGET, 10);
        assert!(outcome.is_some(), "must return an outcome");
        assert!(
            outcome.unwrap().result.is_ok(),
            "first set_nonce must succeed"
        );

        // Attempt to set nonce to 5 (< 10).
        let outcome = nonce::set_nonce(&mut ctx, NONCE_TARGET, 5);
        assert!(outcome.is_some(), "must return an outcome");
        assert!(
            !outcome.unwrap().result.is_ok(),
            "set_nonce to lower value must revert"
        );
    }

    /// vm.setNonce with the same value as the current nonce must succeed
    /// (idempotent).
    #[test]
    fn set_nonce_same_value_succeeds() {
        let mut ctx = revm::context::Context::mainnet();
        let outcome = nonce::set_nonce(&mut ctx, NONCE_TARGET, 42);
        assert!(outcome.is_some() && outcome.unwrap().result.is_ok());

        let outcome = nonce::set_nonce(&mut ctx, NONCE_TARGET, 42);
        assert!(outcome.is_some() && outcome.unwrap().result.is_ok());

        let outcome = nonce::get_nonce(&mut ctx, NONCE_TARGET);
        let decoded =
            U256::from_be_bytes::<32>(outcome.unwrap().result.output.as_ref().try_into().unwrap());
        assert_eq!(decoded, U256::from_limbs([42, 0, 0, 0]));
    }

    /// vm.getNonce on an unknown account must return zero.
    #[test]
    fn get_nonce_returns_zero_for_unknown_account() {
        let mut ctx = revm::context::Context::mainnet();
        let unknown = address!("0x00000000000000000000000000000000000000ab");
        let outcome = nonce::get_nonce(&mut ctx, unknown);
        assert!(outcome.is_some(), "must return an outcome");
        let decoded =
            U256::from_be_bytes::<32>(outcome.unwrap().result.output.as_ref().try_into().unwrap());
        assert_eq!(decoded, U256::ZERO, "unknown account nonce must be 0");
    }

    /// The nonce set during setup must be readable via the contract getter
    /// in a later transaction, proving cross-transaction persistence.
    #[test]
    fn set_nonce_persists_in_storage() {
        let (mut chain, target) = deploy_and_setup();
        let decoded: U256 =
            call_uint256_getter!(&mut chain, target, NonceTarget::getStoredNonceCall);
        assert_eq!(
            decoded, EXPECTED_NONCE,
            "stored nonce must match the value set in setup"
        );
    }

    /// vm.setNonce with the same value twice in one tx must yield the same
    /// nonce reading, proving determinism.
    #[test]
    fn set_nonce_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = NonceTarget::callSetNonceSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callSetNonceSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = NonceTarget::callSetNonceSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first, ret.second,
            "same setNonce value must give identical readings"
        );
        assert_eq!(ret.first, EXPECTED_NONCE);
    }

    /// vm.setNonce with different values interleaved must produce distinct
    /// nonce readings, proving the cheatcode is stateful.
    #[test]
    fn set_nonce_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = NonceTarget::callSetNonceSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callSetNonceSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = NonceTarget::callSetNonceSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.first, U256::from(1), "first vm.setNonce(1) must read 1");
        assert_eq!(
            ret.second, EXPECTED_NONCE,
            "second vm.setNonce(42) must read 42"
        );
        assert_eq!(
            ret.third,
            U256::from(100),
            "third vm.setNonce(100) must read 100"
        );
    }

    /// vm.setNonce must work correctly when combined with vm.deal in the same tx.
    #[test]
    fn set_nonce_interacts_with_deal() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = NonceTarget::callSetNonceAndDealCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callSetNonceAndDeal() must succeed");
        let output = result.output.expect("must return output");
        let ret = NonceTarget::callSetNonceAndDealCall::abi_decode_returns(&output).unwrap();
        assert_eq!(ret.nonce, EXPECTED_NONCE, "nonce must match setNonce value");
        assert_eq!(
            ret.balance,
            expected_balance(),
            "balance must match dealt value"
        );
    }

    /// Setting nonce lower than current via a contract call must revert the
    /// whole transaction, matching Foundry behavior.
    #[test]
    fn set_nonce_lower_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = NonceTarget::callSetNonceAndRevertLowNonceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(
            !result.success,
            "callSetNonceAndRevertLowNonce() must revert"
        );
    }

    /// Invariant must pass immediately after setup (fuzzing baseline).
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = NonceTarget::invariant_nonceCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    /// This proves vm.setNonce stays consistent across multiple transactions
    /// and that invariants correctly observe the mutated state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate nonce via a sequence that ends on a different value.
        let calldata = NonceTarget::callSetNonceSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callSetNonceSequence must succeed");

        // Restore the expected nonce with an action.
        let calldata = NonceTarget::actionSetNonceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionSetNonce must succeed");

        // Invariant must pass after the action restored state.
        let calldata = NonceTarget::invariant_nonceCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.setNonce(42) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn set_nonce_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = NonceTarget::actionSetNonceCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionSetNonce must succeed");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, NonceTarget::getStoredNonceCall);
        assert_eq!(
            stored, EXPECTED_NONCE,
            "stored nonce must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionSetNonce must succeed on second call");
        let stored: U256 =
            call_uint256_getter!(&mut chain, target, NonceTarget::getStoredNonceCall);
        assert_eq!(
            stored, EXPECTED_NONCE,
            "stored nonce must still match after second action"
        );
    }

    /// vm.getNonce must return the expected nonce when queried directly
    /// through the contract after setup.
    #[test]
    fn get_nonce_external_returns_expected() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = NonceTarget::getNonceExternalCall::new((NONCE_TARGET,)).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getNonceExternal must succeed");
        let output = result.output.expect("must return output");
        let decoded = NonceTarget::getNonceExternalCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            decoded, EXPECTED_NONCE,
            "getNonceExternal must return the nonce set in setup"
        );
    }
}
