//! Test-only EVM harness for cheatcode unit tests.
//!
//! Provides a minimal EVM builder that does not depend on `chain`.

use anyhow::{Context, Result};
use revm::{
    MainBuilder, MainContext,
    context::TxEnv,
    database::InMemoryDB,
    inspector::InspectCommitEvm,
    primitives::{Address, Bytes, TxKind, U256},
    state::AccountInfo,
};

use crate::vm::ExecutionState;
use crate::vm::VM_ADDRESS;
use crate::vm::inspector::CheatcodeInspector;

/// Run a single cheatcode call against the VM precompile and return the
/// execution result plus the updated [`ExecutionState`].
pub fn run_cheatcode(
    caller: Address,
    input: Bytes,
    exec_state: ExecutionState,
) -> Result<(revm::context::result::ExecutionResult, ExecutionState)> {
    let mut db = InMemoryDB::default();
    db.insert_account_info(
        caller,
        AccountInfo {
            balance: U256::MAX,
            nonce: 0,
            code_hash: revm::primitives::KECCAK_EMPTY,
            code: None,
            account_id: None,
        },
    );
    let vm_code = revm::bytecode::Bytecode::new_raw(Bytes::from_static(&[0x00]));
    db.insert_account_info(
        VM_ADDRESS,
        AccountInfo {
            balance: U256::ZERO,
            nonce: 0,
            code_hash: vm_code.hash_slow(),
            code: Some(vm_code),
            account_id: None,
        },
    );

    let mut ctx = revm::context::Context::mainnet().with_db(db);
    ctx.block.gas_limit = u64::MAX;
    ctx.cfg.tx_gas_limit_cap = Some(u64::MAX);
    let inspector = CheatcodeInspector::from_state(exec_state);
    let mut evm = ctx.build_mainnet_with_inspector(inspector);

    let tx = TxEnv {
        caller,
        kind: TxKind::Call(VM_ADDRESS),
        data: input,
        gas_limit: u64::MAX,
        ..Default::default()
    };

    let result = evm.inspect_tx_commit(tx).context("evm execution failed")?;
    Ok((result, evm.inspector.state))
}
