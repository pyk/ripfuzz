//! Chain initialization: deployment of the target contract.

use std::path::PathBuf;

use revm::{
    Database, MainBuilder, MainContext,
    context::result::{ExecutionResult, HaltReason},
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
// address(uint160(uint256(keccak256("raptor deployer"))))
pub const DEFAULT_DEPLOYER: Address = Address::new([
    0xc3, 0x42, 0x96, 0x17, 0x5b, 0x9e, 0x78, 0xf6, 0x6e, 0xdb, 0xea, 0xeb, 0x7a, 0xce, 0xa4, 0xc6,
    0x15, 0xc0, 0x92, 0xe1,
]);
pub const GAS_LIMIT: u64 = 16_777_216;
pub const DEFAULT_TX_GAS_LIMIT: u64 = 12_500_000;

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
        ExecutionResult::Success { .. } => "contract returned no address".into(),
        ExecutionResult::Revert { output, .. } => {
            if let Some(reason) = decode_solidity_error(output) {
                format!("reverted with '{reason}'")
            } else {
                format!("reverted (output: 0x{})", hex::encode(output))
            }
        }
        ExecutionResult::Halt { reason, .. } => match reason {
            HaltReason::OutOfGas(_) => {
                format!("ran out of gas ({reason}) -- try increasing --block-gas-limit")
            }
            _ => format!("halted: {reason}"),
        },
    }
}

/// Decode a Solidity `Error(string)` revert payload.
pub(crate) fn decode_solidity_error(output: &Bytes) -> Option<String> {
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
    deployer: Address,
    block_gas_limit: u64,
) -> Result<(Address, ChainState), ChainInitError> {
    let mut db = InMemoryDB::default();

    db.insert_account_info(
        deployer,
        AccountInfo {
            balance: U256::from_str_radix("ffffffffffffffffffffffffffffffff", 16)
                .unwrap_or(U256::MAX),
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
        caller: deployer,
        kind: TxKind::Create,
        data: target.initcode.clone(),
        gas_limit: block_gas_limit,
        value: deploy_value,
        ..Default::default()
    };

    let result = evm.inspect_tx_commit(tx).map_err(|e| {
        let err: anyhow::Error = e.into();
        let msg = format!("{err}");
        if msg.contains("gas limit") || msg.contains("gas cost") {
            anyhow::anyhow!("{msg} -- try increasing --block-gas-limit")
        } else {
            err
        }
    })?;
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
        .basic(deployer)
        .unwrap_or_default()
        .unwrap_or_default()
        .nonce;
    state.known_contracts.insert(
        contract_address,
        (target.contract_name.clone(), target.abi.clone()),
    );

    Ok((contract_address, state))
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_DEPLOYER;
    use alloy_primitives::utils::keccak256;

    #[test]
    fn default_deployer_matches_raptor_deployer_string() {
        let hash = keccak256(b"raptor deployer");
        let expected = revm::primitives::Address::from_word(hash);
        assert_eq!(expected, DEFAULT_DEPLOYER);
    }
}
