//! Chain setup: optional `setup()` call after deployment.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Context as AnyhowContext;
use revm::{
    Database, MainBuilder, MainContext,
    context::{Context, TxEnv},
    inspector::InspectCommitEvm,
    primitives::{Bytes, TxKind},
};

use tracing::{error, info, instrument, trace};

use crate::chain::base_state::BaseState;
use crate::chain::error::ChainSetupError;
use crate::chain::inspectors::{
    InspectorTuple, MaybeTrace, coverage::CoverageInspector, trace::TraceInspector,
};

const SETUP_SELECTOR: [u8; 4] = [0xba, 0x0b, 0xba, 0x40];

/// Run `setup()` if present and return the updated base state.
#[instrument(skip(state), fields(contract = %contract_address), err)]
pub fn setup(
    state: BaseState,
    contract_address: revm::primitives::Address,
    abi: &alloy_json_abi::JsonAbi,
    initcode_map: &HashMap<Bytes, (String, alloy_json_abi::JsonAbi)>,
    deployer: revm::primitives::Address,
) -> Result<BaseState, ChainSetupError> {
    let t0 = std::time::Instant::now();
    let has_setup = abi.functions().any(|f| f.selector() == SETUP_SELECTOR);
    if !has_setup {
        trace!("no setup function found");
        return Ok(state);
    }

    let mut db = state.db;
    let nonce = crate::result_to_option(db.basic(deployer))
        .flatten()
        .map(|info| info.nonce)
        .unwrap_or(0);

    let mut trace_inspector = TraceInspector::new(initcode_map.clone());
    if let Some((name, contract_abi)) = state.known_contracts.get(&contract_address) {
        trace_inspector.register_contract(contract_address, name, contract_abi.clone());
    }

    let shared_labels = Arc::new(RwLock::new(state.labels.clone()));
    trace_inspector.set_shared_labels(Arc::clone(&shared_labels));

    let exec_state = crate::evm::cheatcode::ExecutionState {
        project_root: state.project_root.clone(),
        ffi_enabled: state.ffi_enabled,
        compiled_contracts: state.compiled_contracts.clone(),
        labels: state.labels.clone(),
        prank: state.prank.clone(),
        block: state.block_overrides,
        eth_deals: Vec::new(),
        nonce_changes: Vec::new(),
    };
    let cheatcode_inspector =
        crate::evm::cheatcode::inspector::CheatcodeInspector::from_state(exec_state)
            .with_shared_labels(shared_labels);

    let inspector = InspectorTuple::new(
        CoverageInspector::new(),
        MaybeTrace(Some(trace_inspector)),
        cheatcode_inspector,
    );
    let mut ctx = Context::mainnet().with_db(db);
    ctx.block.gas_limit = u64::MAX;
    ctx.cfg.tx_gas_limit_cap = Some(u64::MAX);
    let mut evm = ctx.build_mainnet_with_inspector(inspector);

    let setup_tx = TxEnv {
        caller: deployer,
        kind: TxKind::Call(contract_address),
        data: Bytes::copy_from_slice(&SETUP_SELECTOR),
        gas_limit: u64::MAX,
        nonce,
        ..Default::default()
    };
    let setup_result = evm
        .inspect_tx_commit(setup_tx)
        .map_err(|e| -> anyhow::Error { e.into() })?;
    if !setup_result.is_success() {
        let reason = crate::chain::init::extract_deployment_error(&setup_result);
        let trace = evm
            .inspector
            .1
            .0
            .context("trace inspector missing")?
            .into_trace_tree()
            .format();
        error!(%reason, "setup failed");
        return Err(ChainSetupError::SetupFailed { reason, trace });
    }
    let elapsed = t0.elapsed();
    info!(time_ms = elapsed.as_millis(), "Ran setup");

    let mut new_state = crate::chain::base_state::BaseState::new(evm.ctx.journaled_state.database);
    new_state.caller_nonce = new_state
        .db
        .basic(deployer)
        .unwrap_or_default()
        .unwrap_or_default()
        .nonce;
    // Persistent config copied straight through.
    new_state.project_root = state.project_root;
    new_state.ffi_enabled = state.ffi_enabled;
    new_state.compiled_contracts = state.compiled_contracts;
    // Committed cheatcode state extracted from inspector.
    let inspector = evm.inspector.2;
    new_state.labels = inspector.state.labels;
    new_state.prank = inspector.state.prank;
    new_state.block_overrides = inspector.state.block;
    // eth_deals and nonce_changes are NOT copied (dropped with the inspector).
    if let Some(ts) = new_state.block_overrides.timestamp {
        new_state.block_timestamp = u64::try_from(ts).unwrap_or(u64::MAX);
    }
    if let Some(num) = new_state.block_overrides.number {
        new_state.block_number = u64::try_from(num).unwrap_or(u64::MAX);
    }
    Ok(new_state)
}
