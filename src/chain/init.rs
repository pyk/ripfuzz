//! Chain initialization: deployment of the target contract.

use std::path::PathBuf;

use revm::{
    Database, MainBuilder, MainContext,
    context::{Context, TxEnv},
    database::InMemoryDB,
    inspector::InspectCommitEvm,
    primitives::{Address, Bytes, KECCAK_EMPTY, TxKind, U256},
    state::AccountInfo,
};

use tracing::{error, info, instrument};

use crate::chain::error::ChainInitError;
use crate::chain::inspectors::trace::VM_ADDRESS;
use crate::chain::state::ChainState;
use crate::contract::ContractArtifact;

pub const CALLER: Address = Address::new([0xde; 20]);
pub const GAS_LIMIT: u64 = 16_777_216;

/// Insert a dummy VM contract into the database so Solidity's
/// `extcodesize` check passes when a target calls Foundry cheatcodes.
pub(crate) fn insert_foundry_vm(db: &mut InMemoryDB) {
    let vm_code = revm::bytecode::Bytecode::new_raw(revm::primitives::Bytes::from_static(&[0x00]));
    db.insert_account_info(
        VM_ADDRESS,
        revm::state::AccountInfo {
            balance: revm::primitives::U256::ZERO,
            nonce: 0,
            code_hash: vm_code.hash_slow(),
            code: Some(vm_code),
            account_id: None,
        },
    );
}

/// Extract a human-readable error message from a failed deployment result.
pub(crate) fn extract_deployment_error(result: &revm::context::result::ExecutionResult) -> String {
    match result {
        revm::context::result::ExecutionResult::Success { .. } => {
            "contract returned no address".into()
        }
        revm::context::result::ExecutionResult::Revert { output, .. } => {
            if let Some(reason) = decode_solidity_error(output) {
                format!("reverted with '{reason}'")
            } else {
                format!("reverted (output: 0x{})", hex::encode(output))
            }
        }
        revm::context::result::ExecutionResult::Halt { reason, .. } => {
            format!("halted: {reason}")
        }
    }
}

/// Decode a Solidity `Error(string)` revert payload.
fn decode_solidity_error(output: &Bytes) -> Option<String> {
    if output.len() < 4 || output[..4] != ERROR_SELECTOR {
        return None;
    }

    let string_type = alloy_dyn_abi::DynSolType::String;
    let decoded = crate::result_to_option(string_type.abi_decode_params(&output[4..]))?;

    match decoded {
        alloy_dyn_abi::DynSolValue::String(s) => Some(s),
        _ => None,
    }
}

/// Solidity `Error(string)` selector: `keccak256("Error(string)")[:4]`
const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

/// Deploy a contract from an artifact and return the post-deployment chain state.
#[instrument(skip(target), fields(contract = %target.contract_name), err)]
pub fn initialize(
    target: &ContractArtifact,
    project_root: PathBuf,
    ffi_enabled: bool,
    deploy_value: U256,
) -> Result<(Address, ChainState), ChainInitError> {
    let mut db = InMemoryDB::default();

    db.insert_account_info(
        CALLER,
        AccountInfo {
            balance: U256::from(u128::MAX),
            nonce: 0,
            code_hash: KECCAK_EMPTY,
            code: None,
            account_id: None,
        },
    );
    insert_foundry_vm(&mut db);

    let inspector =
        crate::chain::inspectors::trace::TraceInspector::new(target.initcode_map.clone());
    let ctx = Context::mainnet().with_db(db);
    let mut evm = ctx.build_mainnet_with_inspector(inspector);

    let tx = TxEnv {
        caller: CALLER,
        kind: TxKind::Create,
        data: target.initcode.clone(),
        gas_limit: GAS_LIMIT,
        value: deploy_value,
        ..Default::default()
    };

    let result = evm
        .inspect_tx_commit(tx)
        .map_err(|e| -> anyhow::Error { e.into() })?;
    let contract_address = result.created_address().ok_or_else(|| {
        let reason = extract_deployment_error(&result);
        let trace = evm.inspector.into_trace_tree().format();
        error!(%reason, "deployment failed");
        ChainInitError::DeploymentFailed { reason, trace }
    })?;
    info!(%contract_address, "contract deployed");

    let deployed_db = evm.ctx.journaled_state.database;
    let mut state = ChainState::new(deployed_db);
    state.cheatcodes.project_root = project_root;
    state.cheatcodes.ffi_enabled = ffi_enabled;
    state.caller_nonce = state
        .db
        .basic(CALLER)
        .unwrap_or_default()
        .unwrap_or_default()
        .nonce;
    state.known_contracts.insert(
        contract_address,
        (target.contract_name.clone(), target.abi.clone()),
    );

    Ok((contract_address, state))
}
