//! Chain setup: optional `setUp()` call after deployment.

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

use crate::chain::error::ChainSetupError;
use crate::chain::init::GAS_LIMIT;
use crate::chain::inspectors::{
    InspectorTuple, MaybeTrace, coverage::CoverageInspector, trace::TraceInspector,
};
use crate::chain::state::ChainState;

const SETUP_SELECTOR: [u8; 4] = [0x0a, 0x92, 0x54, 0xe4];

/// Run `setUp()` if present and return the updated chain state.
#[instrument(skip(state), fields(contract = %contract_address), err)]
pub fn setup(
    state: ChainState,
    contract_address: revm::primitives::Address,
    abi: &alloy_json_abi::JsonAbi,
    initcode_map: &HashMap<Bytes, (String, alloy_json_abi::JsonAbi)>,
    deployer: revm::primitives::Address,
) -> Result<ChainState, ChainSetupError> {
    let has_setup = abi.functions().any(|f| f.selector() == SETUP_SELECTOR);
    if !has_setup {
        trace!("no setUp function found");
        return Ok(state);
    }

    // Preserve compiled-contract map so vm.getCode works across setUp.
    let compiled_contracts = state.cheatcodes.compiled_contracts.clone();

    let mut db = state.db;
    let nonce = crate::result_to_option(db.basic(deployer))
        .flatten()
        .map(|info| info.nonce)
        .unwrap_or(0);

    let mut trace_inspector = TraceInspector::new(initcode_map.clone());
    if let Some((name, contract_abi)) = state.known_contracts.get(&contract_address) {
        trace_inspector.register_contract(contract_address, name, contract_abi.clone());
    }

    let shared_labels = Arc::new(RwLock::new(state.cheatcodes.labels.clone()));
    trace_inspector.set_shared_labels(Arc::clone(&shared_labels));
    let cheatcode_inspector =
        crate::chain::inspectors::cheatcode::CheatcodeInspector::from_state(state.cheatcodes)
            .with_shared_labels(shared_labels);

    let inspector = InspectorTuple::new(
        CoverageInspector::new(),
        MaybeTrace(Some(trace_inspector)),
        cheatcode_inspector,
    );
    let ctx = Context::mainnet().with_db(db);
    let mut evm = ctx.build_mainnet_with_inspector(inspector);

    let setup_tx = TxEnv {
        caller: deployer,
        kind: TxKind::Call(contract_address),
        data: revm::primitives::Bytes::copy_from_slice(&SETUP_SELECTOR),
        gas_limit: GAS_LIMIT,
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
        error!(%reason, "setUp failed");
        return Err(ChainSetupError::SetupFailed { reason, trace });
    }
    info!("setUp succeeded");

    let mut new_state = crate::chain::state::ChainState::new(evm.ctx.journaled_state.database);
    new_state.caller_nonce = new_state
        .db
        .basic(deployer)
        .unwrap_or_default()
        .unwrap_or_default()
        .nonce;
    // Persist cheatcode state from setUp so it carries into each sequence.
    let cheat_inspector = evm.inspector.2;
    new_state.cheatcodes = cheat_inspector.state;
    // setUp deals and nonce changes are committed to the base state; clear
    // records so they are not rolled back on a later reverted call in a
    // sequence.
    new_state.cheatcodes.eth_deals.clear();
    new_state.cheatcodes.nonce_changes.clear();
    // Restore compiled-contract map so vm.getCode keeps working.
    new_state.cheatcodes.compiled_contracts = compiled_contracts;
    // Persist block context set during setUp so sequences start at the
    // warped / rolled values.
    if let Some(ts) = new_state.cheatcodes.block.timestamp {
        new_state.block_timestamp = u64::try_from(ts).unwrap_or(u64::MAX);
    }
    if let Some(num) = new_state.cheatcodes.block.number {
        new_state.block_number = u64::try_from(num).unwrap_or(u64::MAX);
    }
    Ok(new_state)
}
