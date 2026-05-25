//! `store` / `load` cheatcodes.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Address, U256},
};

use crate::evm::cheatcode::outcome;

pub fn store<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    slot: [u8; 32],
    value: [u8; 32],
) -> Option<revm::interpreter::CallOutcome> {
    if ctx.journal().precompile_addresses().contains(&addr) {
        return Some(outcome::revert("store: cannot write to precompile"));
    }
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    ctx.journal_mut()
        .sstore(addr, U256::from_be_bytes(slot), U256::from_be_bytes(value))
        .map_err(|e| format!("failed to store storage slot: {e:?}"))
        .ok()?;
    Some(outcome::success())
}

pub fn load<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    slot: [u8; 32],
) -> Option<revm::interpreter::CallOutcome> {
    if ctx.journal().precompile_addresses().contains(&addr) {
        return Some(outcome::revert("load: cannot read from precompile"));
    }
    let value = match ctx.journal_mut().load_account_mut(addr) {
        Ok(mut s) => s
            .data
            .sload(U256::from_be_bytes(slot), false)
            .ok()
            .map(|r| r.data.present_value)
            .unwrap_or(U256::ZERO),
        Err(_) => U256::ZERO,
    };
    Some(outcome::success_bytes(value.to_be_bytes_vec()))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::evm::chain::{Chain, Config, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode;
    use crate::evm::cheatcode::calls::storage;
    use crate::evm::result::TransactionResult;

    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface StorageTarget {
            function getStoredValue() external view returns (bytes32);
            function getLoadedValue() external view returns (bytes32);
            function getEmptySlotValue() external view returns (bytes32);
            function callStoreSameValueTwice() external returns (bytes32 first, bytes32 second);
            function callStoreSequence() external returns (bytes32 first, bytes32 second, bytes32 third);
            function callStoreAndWarp() external returns (bytes32 value, uint256 timestamp);
            function callStoreToPrecompile() external;
            function callLoadFromPrecompile() external view;
            function setup() external;
            function actionStore() external;
            function invariant_storage() external view;
        }
    }

    const TARGET_ADDR: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");
    const EXPECTED_VALUE: [u8; 32] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0x2a,
    ];

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    /// Deploy the fixture and run its `setup` function.
    fn deploy_and_setup() -> (Chain, Address) {
        let contract = load_fixture("src/StorageTarget.sol:StorageTarget");
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

    /// Call a view/pure function that returns a single `bytes32` and decode it.
    macro_rules! call_bytes32_getter {
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

    // -----------------------------------------------------------------
    // Handler-level (direct Rust unit tests)
    // -----------------------------------------------------------------

    /// vm.store must succeed for a valid address and slot.
    #[test]
    fn store_sets_value_without_reverting() {
        let mut ctx = revm::context::Context::mainnet();
        let slot = [0u8; 32];
        let outcome = storage::store(&mut ctx, TARGET_ADDR, slot, EXPECTED_VALUE);
        assert!(outcome.is_some(), "must return an outcome");
        assert!(outcome.unwrap().result.is_ok(), "vm.store must succeed");
    }

    /// vm.load from an uninitialized slot must return zero.
    #[test]
    fn load_uninitialized_slot_returns_zero() {
        let mut ctx = revm::context::Context::mainnet();
        let slot = [0u8; 32];
        let outcome = storage::load(&mut ctx, TARGET_ADDR, slot);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.load must succeed");
        assert_eq!(
            outcome.result.output.as_ref(),
            [0u8; 32],
            "uninitialized slot must return zero"
        );
    }

    /// vm.store followed by vm.load must return the stored value.
    #[test]
    fn store_and_load_roundtrip() {
        let mut ctx = revm::context::Context::mainnet();
        let slot = [0u8; 32];
        let store_outcome = storage::store(&mut ctx, TARGET_ADDR, slot, EXPECTED_VALUE);
        assert!(store_outcome.is_some(), "store must return an outcome");
        assert!(store_outcome.unwrap().result.is_ok(), "store must succeed");

        let load_outcome = storage::load(&mut ctx, TARGET_ADDR, slot);
        assert!(load_outcome.is_some(), "load must return an outcome");
        let load_outcome = load_outcome.unwrap();
        assert!(load_outcome.result.is_ok(), "load must succeed");
        assert_eq!(
            load_outcome.result.output.as_ref(),
            EXPECTED_VALUE.as_slice(),
            "load must return the stored value"
        );
    }

    /// vm.load from an unknown address must return zero.
    #[test]
    fn load_from_unknown_address_returns_zero() {
        let mut ctx = revm::context::Context::mainnet();
        let unknown = address!("0x00000000000000000000000000000000000000ab");
        let slot = [0u8; 32];
        let outcome = storage::load(&mut ctx, unknown, slot);
        assert!(outcome.is_some(), "must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "vm.load must succeed");
        assert_eq!(
            outcome.result.output.as_ref(),
            [0u8; 32],
            "load from unknown address must return zero"
        );
    }

    /// vm.store to an unknown address must succeed and the value must be
    /// readable afterwards.
    #[test]
    fn store_to_unknown_address_works() {
        let mut ctx = revm::context::Context::mainnet();
        let unknown = address!("0x00000000000000000000000000000000000000ab");
        let slot = [0u8; 32];
        let outcome = storage::store(&mut ctx, unknown, slot, EXPECTED_VALUE);
        assert!(outcome.is_some(), "store must return an outcome");
        assert!(outcome.unwrap().result.is_ok(), "store must succeed");

        let outcome = storage::load(&mut ctx, unknown, slot);
        assert!(outcome.is_some(), "load must return an outcome");
        let outcome = outcome.unwrap();
        assert!(outcome.result.is_ok(), "load must succeed");
        assert_eq!(
            outcome.result.output.as_ref(),
            EXPECTED_VALUE.as_slice(),
            "load must return the stored value"
        );
    }

    // -----------------------------------------------------------------
    // Basic contract-path integration
    // -----------------------------------------------------------------

    /// The value stored during setup must be readable via the contract getter
    /// in a later transaction, proving persistence.
    #[test]
    fn storage_persists_across_transactions() {
        let (mut chain, target) = deploy_and_setup();
        let decoded = call_bytes32_getter!(&mut chain, target, StorageTarget::getStoredValueCall);
        assert_eq!(
            decoded.as_slice(),
            EXPECTED_VALUE.as_slice(),
            "stored value must persist across transactions"
        );
    }

    /// vm.load must return the same value that was stored during setup.
    #[test]
    fn loaded_value_matches_stored_value() {
        let (mut chain, target) = deploy_and_setup();
        let stored = call_bytes32_getter!(&mut chain, target, StorageTarget::getStoredValueCall);

        let calldata = StorageTarget::getLoadedValueCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getLoadedValue must succeed");
        let output = result.output.expect("must return output");
        let loaded = StorageTarget::getLoadedValueCall::abi_decode_returns(&output).unwrap();

        assert_eq!(
            stored.as_slice(),
            loaded.as_slice(),
            "vm.load must return the same value stored in setup"
        );
        assert_eq!(stored.as_slice(), EXPECTED_VALUE.as_slice());
    }

    /// An empty slot must return zero when queried through the contract.
    #[test]
    fn load_empty_slot_returns_zero_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = StorageTarget::getEmptySlotValueCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "getEmptySlotValue must succeed");
        let output = result.output.expect("must return output");
        let decoded = StorageTarget::getEmptySlotValueCall::abi_decode_returns(&output).unwrap();
        assert_eq!(decoded.as_slice(), [0u8; 32], "empty slot must return zero");
    }

    /// vm.store with the same value twice in one tx must yield the same
    /// reading, proving determinism.
    #[test]
    fn store_same_value_twice_in_sequence_is_deterministic() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = StorageTarget::callStoreSameValueTwiceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStoreSameValueTwice() must succeed");
        let output = result.output.expect("must return output");
        let ret = StorageTarget::callStoreSameValueTwiceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first.as_slice(),
            ret.second.as_slice(),
            "same store value must give identical load readings"
        );
        assert_eq!(ret.first.as_slice(), EXPECTED_VALUE.as_slice());
    }

    /// vm.store with different values interleaved must produce distinct
    /// readings, proving the cheatcode is stateful.
    #[test]
    fn store_sequence_returns_consistent_values() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = StorageTarget::callStoreSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStoreSequence() must succeed");
        let output = result.output.expect("must return output");
        let ret = StorageTarget::callStoreSequenceCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.first.as_slice(),
            [
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 1
            ]
            .as_slice(),
            "first vm.store(1) must read 1"
        );
        assert_eq!(
            ret.second.as_slice(),
            EXPECTED_VALUE.as_slice(),
            "second vm.store(42) must read 42"
        );
        assert_eq!(
            ret.third.as_slice(),
            [
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 2
            ]
            .as_slice(),
            "third vm.store(2) must read 2"
        );
    }

    /// vm.store must work correctly when combined with vm.warp in the same tx.
    #[test]
    fn store_interacts_with_warp() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = StorageTarget::callStoreAndWarpCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStoreAndWarp() must succeed");
        let output = result.output.expect("must return output");
        let ret = StorageTarget::callStoreAndWarpCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            ret.value.as_slice(),
            EXPECTED_VALUE.as_slice(),
            "loaded value must match stored value"
        );
        assert_eq!(
            ret.timestamp,
            U256::from(1_234_567_890u64),
            "timestamp must match warped value"
        );
    }

    /// vm.store to a precompile must revert when called through the contract.
    #[test]
    fn store_to_precompile_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = StorageTarget::callStoreToPrecompileCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(!result.success, "callStoreToPrecompile() must revert");
    }

    /// vm.load from a precompile must revert when called through the contract.
    #[test]
    fn load_from_precompile_reverts_via_contract() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = StorageTarget::callLoadFromPrecompileCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(!result.success, "callLoadFromPrecompile() must revert");
    }

    // -----------------------------------------------------------------
    // Invariants (fuzzing baseline)
    // -----------------------------------------------------------------

    /// Invariant must pass immediately after setup.
    #[test]
    fn invariant_passes_after_setup() {
        let (mut chain, target) = deploy_and_setup();
        let calldata = StorageTarget::invariant_storageCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after setup");
    }

    /// A fuzz-like sequence of actions followed by invariants must all succeed.
    /// This proves vm.store/load stays deterministic across multiple
    /// transactions and that invariants correctly observe the mutated state.
    #[test]
    fn action_sequence_and_invariants() {
        let (mut chain, target) = deploy_and_setup();

        // Temporarily mutate storage via a sequence that ends on a different value.
        let calldata = StorageTarget::callStoreSequenceCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "callStoreSequence must succeed");

        // Restore the expected value with an action.
        let calldata = StorageTarget::actionStoreCall::new(()).abi_encode();
        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionStore must succeed");

        // Invariant must pass after the action restored state.
        let calldata = StorageTarget::invariant_storageCall::new(()).abi_encode();
        let result = chain
            .call(DEFAULT_DEPLOYER, target, U256::ZERO, Bytes::from(calldata))
            .unwrap();
        assert!(result.success, "invariant must pass after action sequence");
    }

    /// vm.store(expected) must set the same value when called in a separate
    /// transaction after the initial setup, proving cross-transaction determinism.
    #[test]
    fn storage_deterministic_across_transaction_sequence() {
        let (mut chain, target) = deploy_and_setup();

        let calldata = StorageTarget::actionStoreCall::new(()).abi_encode();

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata.clone()),
        );
        assert!(result.success, "actionStore must succeed");
        let stored = call_bytes32_getter!(&mut chain, target, StorageTarget::getStoredValueCall);
        assert_eq!(
            stored.as_slice(),
            EXPECTED_VALUE.as_slice(),
            "stored value must match after first action"
        );

        let result = call_with_cheatcode_inspector(
            &mut chain,
            DEFAULT_DEPLOYER,
            target,
            Bytes::from(calldata),
        );
        assert!(result.success, "actionStore must succeed on second call");
        let stored = call_bytes32_getter!(&mut chain, target, StorageTarget::getStoredValueCall);
        assert_eq!(
            stored.as_slice(),
            EXPECTED_VALUE.as_slice(),
            "stored value must still match after second action"
        );
    }
}
