//! `store` / `load` cheatcodes.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Address, U256},
};

use crate::evm::cheatcode::outcome;
use crate::evm::database::DatabaseExt;

pub fn store<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    slot: [u8; 32],
    value: [u8; 32],
) -> Option<revm::interpreter::CallOutcome>
where
    CTX::Db: DatabaseExt,
{
    if ctx.journal().precompile_addresses().contains(&addr) {
        return Some(outcome::revert("store: cannot write to precompile"));
    }
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    // Pre-populate the database cache with a zero value for this slot
    // so that the subsequent sstore does not trigger an eth_getStorageAt
    // fetch from the fork DB. vm.store is a blind write -- we don't
    // care about the original value.
    let slot_u256 = U256::from_be_bytes(slot);
    let value_u256 = U256::from_be_bytes(value);
    // Ignore errors -- if pre-population fails the sstore will handle it.
    let _ = ctx
        .db_mut()
        .insert_account_storage(addr, slot_u256, U256::ZERO);
    ctx.journal_mut()
        .sstore(addr, slot_u256, value_u256)
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

    use alloy_primitives::{Address, address};
    use alloy_sol_types::SolCall;
    use revm::MainContext;
    use revm::primitives::Bytes;

    use crate::compilers::solc::{Solc, SolcOutput};
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::calls::storage;
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
        interface StorageHarness {
            function setup() external;
            function getLoadedValue() external view returns (bytes32);
            function getEmptySlotValue() external view returns (bytes32);
            function actionRestore() external;
            function actionMutate() external;
            function actionSequence() external;
            function actionStorePrecompile() external;
            function actionLoadPrecompile() external view;
            function invariant_valueMatch() external view;
        }
    }

    const EXPECTED_VALUE: [u8; 32] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0x2a,
    ];

    const TARGET_ADDR: Address = address!("0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF");

    fn load_initcode(id: &str) -> String {
        compile_fixture("fixtures/evm/cheatcodes", id)
            .initcode()
            .unwrap()
            .to_owned()
    }

    fn deploy_and_setup() -> (Chain, Address) {
        let initcode = load_initcode("StorageHarness.sol:StorageHarness");
        let mut chain = Chain::new(ChainConfig::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    // Harness-level unit tests

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

    // Integration tests

    /// `vm.store` used during setup must write the expected value, and
    /// `vm.load` in a later transaction must read it back. The invariant
    /// verifies that the stored state matches the expected canonical value.
    #[test]
    fn setup_stores_and_loads_canonical_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::getLoadedValueCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::invariant_valueMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "getLoadedValue must succeed after setup"
        );
        let output = execution.results[0]
            .output
            .clone()
            .expect("must return output");
        let loaded = StorageHarness::getLoadedValueCall::abi_decode_returns(&output).unwrap();
        assert_eq!(
            loaded.as_slice(),
            EXPECTED_VALUE.as_slice(),
            "vm.load must return the expected value"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass after setup"
        );
    }

    /// Re-storing the canonical value in a later transaction must not corrupt
    /// the expected state. This is the core property a stateful fuzzer relies
    /// on when actions need to restore known storage state.
    #[test]
    fn restore_in_action_preserves_value() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::actionRestoreCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::invariant_valueMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionRestore must succeed");
        assert!(
            execution.results[1].success,
            "invariant must pass after restoring value"
        );
    }

    /// Mutating storage to a non-canonical value must break the invariant.
    /// This proves `vm.store` actually changes observable contract state.
    #[test]
    fn mutate_breaks_invariant() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::actionMutateCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::invariant_valueMatchCall::new(()).abi_encode(),
            )),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionMutate must succeed");
        assert!(
            !execution.results[1].success,
            "invariant must fail after mutating storage"
        );
    }

    /// A single transaction can store and load multiple unrelated slots without
    /// corrupting the canonical slot. This proves `vm.store` is safe to call
    /// repeatedly inside one tx.
    #[test]
    fn sequence_in_single_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::actionSequenceCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::invariant_valueMatchCall::new(()).abi_encode(),
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

    /// `vm.store` to a precompile must revert in a transaction.
    #[test]
    fn store_to_precompile_reverts_in_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            StorageHarness::actionStorePrecompileCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            !execution.results[0].success,
            "vm.store to precompile must revert in a transaction"
        );
    }

    /// `vm.load` from a precompile must revert in a transaction.
    #[test]
    fn load_from_precompile_reverts_in_transaction() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            StorageHarness::actionLoadPrecompileCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            !execution.results[0].success,
            "vm.load from precompile must revert in a transaction"
        );
    }

    /// An uninitialized slot must return zero when read via vm.load in a
    /// transaction.
    #[test]
    fn load_empty_slot_returns_zero() {
        let (mut chain, target) = deploy_and_setup();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            StorageHarness::getEmptySlotValueCall::new(()).abi_encode(),
        ))];

        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(
            execution.results[0].success,
            "getEmptySlotValue must succeed"
        );
        let output = execution.results[0]
            .output
            .clone()
            .expect("must return output");
        let decoded = StorageHarness::getEmptySlotValueCall::abi_decode_returns(&output).unwrap();
        assert_eq!(decoded.as_slice(), [0u8; 32], "empty slot must return zero");
    }

    /// A cloned chain snapshot must produce the same storage when actions are
    /// executed on the clone. This is critical for parallel fuzzing where each
    /// worker starts from a cloned state.
    #[test]
    fn cloned_chain_preserves_storage() {
        let (chain, target) = deploy_and_setup();
        let mut cloned = chain.clone();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::actionRestoreCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::invariant_valueMatchCall::new(()).abi_encode(),
            )),
        ];
        let execution = cloned.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results[0].success,
            "actionRestore must succeed on cloned chain"
        );
        assert!(
            execution.results[1].success,
            "invariant must pass on cloned chain"
        );
    }

    /// Cross-transaction determinism: re-storing in a second `exec` must still
    /// leave the canonical value intact.
    #[test]
    fn deterministic_across_separate_execs() {
        let (mut chain, target) = deploy_and_setup();

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::actionRestoreCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::invariant_valueMatchCall::new(()).abi_encode(),
            )),
        ];
        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results.iter().all(|r| r.success),
            "first exec must succeed"
        );

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::actionRestoreCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                StorageHarness::invariant_valueMatchCall::new(()).abi_encode(),
            )),
        ];
        let execution = chain.exec(&txs).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(
            execution.results.iter().all(|r| r.success),
            "second exec must succeed"
        );
    }
}
