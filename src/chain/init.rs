//! Chain initialization: deployment of the target contract.

use std::path::PathBuf;
use std::sync::Arc;

use revm::{
    Database, MainBuilder, MainContext,
    context::result::ExecutionResult,
    context::{Context, TxEnv},
    inspector::InspectCommitEvm,
    primitives::{Address, Bytes, KECCAK_EMPTY, TxKind, U256},
    state::AccountInfo,
};

use tracing::{error, info, instrument};

use crate::chain::error::ChainInitError;
use crate::chain::state::{ChainDatabase, ChainState};
use crate::contract::ContractArtifact;
use crate::vm::VM_ADDRESS;

pub const CALLER: Address = Address::new([0xde; 20]);
// address(uint160(uint256(keccak256("raptor deployer"))))
pub const DEFAULT_DEPLOYER: Address = Address::new([
    0xc3, 0x42, 0x96, 0x17, 0x5b, 0x9e, 0x78, 0xf6, 0x6e, 0xdb, 0xea, 0xeb, 0x7a, 0xce, 0xa4, 0xc6,
    0x15, 0xc0, 0x92, 0xe1,
]);

/// Insert a dummy VM contract into the database so Solidity's
/// `extcodesize` check passes when a target calls raptor cheatcodes.
pub fn insert_raptor_vm(db: &mut ChainDatabase) {
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
pub fn extract_deployment_error(result: &revm::context::result::ExecutionResult) -> String {
    match result {
        ExecutionResult::Success { .. } => "contract returned no address".into(),
        ExecutionResult::Revert { output, .. } => {
            if let Some(reason) = decode_solidity_error(output) {
                format!("reverted with '{reason}'")
            } else {
                format!("reverted (output: 0x{})", hex::encode(output))
            }
        }
        ExecutionResult::Halt { reason, .. } => format!("halted: {reason}"),
    }
}

/// Decode a Solidity `Error(string)` revert payload.
pub fn decode_solidity_error(output: &Bytes) -> Option<String> {
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
    rpc: Option<&Arc<dyn crate::rpc::RpcClient>>,
    fork_config: Option<&crate::chain::fork::ForkConfig>,
) -> Result<(Address, ChainState), ChainInitError> {
    let t0 = std::time::Instant::now();
    let mut db = if let (Some(rpc), Some(config)) = (rpc, fork_config) {
        let backend = crate::chain::fork::ForkBackend::new(
            Arc::clone(rpc),
            config.block_number,
            &project_root,
        )
        .map_err(|e| ChainInitError::Other(anyhow::anyhow!("fork initialization failed: {e}")))?;
        crate::chain::fork::ForkDatabase::new(backend)
    } else {
        crate::chain::fork::ForkDatabase::new(crate::chain::fork::ForkBackend::empty())
    };

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
    insert_raptor_vm(&mut db);

    let inspector =
        crate::chain::inspectors::trace::TraceInspector::new(target.initcode_map.clone());
    let mut ctx = Context::mainnet().with_db(db);
    ctx.block.gas_limit = u64::MAX;
    ctx.cfg.tx_gas_limit_cap = Some(u64::MAX);
    let mut evm = ctx.build_mainnet_with_inspector(inspector);

    let tx = TxEnv {
        caller: deployer,
        kind: TxKind::Create,
        data: target.initcode.clone(),
        gas_limit: u64::MAX,
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
    let elapsed = t0.elapsed();
    info!(target: "raptor::user", time_ms = elapsed.as_millis(), "Deployed target contract");

    let deployed_db = evm.ctx.journaled_state.database;
    let mut state = ChainState::new(deployed_db);
    state.vm.project_root = project_root;
    state.vm.ffi_enabled = ffi_enabled;
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
