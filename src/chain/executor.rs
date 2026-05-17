//! Sequence execution: runs a call sequence against a cloned chain state.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use alloy_json_abi::JsonAbi;
use revm::{
    MainBuilder, MainContext,
    context::{Context, TxEnv},
    inspector::{InspectCommitEvm, NoOpInspector},
    primitives::{Bytes, TxKind, U256},
};

use tracing::trace;

use crate::chain::{
    ChainConfig,
    error::ChainExecutionError,
    inspectors::{InspectorTuple, MaybeTrace, coverage::CoverageInspector, trace::TraceInspector},
    output::{CallMeta, ExecutionOutput, PropertyResult},
    state::ChainState,
};
use crate::corpus::Call;

/// Options controlling expensive execution features.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutionOptions {
    /// Enable call-trace collection.
    pub trace: bool,
}

/// Execute a call sequence against a cloned post-setup state.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    state: &ChainState,
    contract_address: revm::primitives::Address,
    properties: &[([u8; 4], String)],
    contract_abi: &alloy_json_abi::JsonAbi,
    config: &ChainConfig,
    initcode_map: &HashMap<Bytes, (String, JsonAbi)>,
    calls: &[Call],
    opts: ExecutionOptions,
) -> Result<ExecutionOutput, ChainExecutionError> {
    // 1. Clone the immutable post-setup state.
    let mut local_state = state.clone();

    // 2. Build inspectors (owned, no external mutable references).
    let coverage_inspector = CoverageInspector::new();
    let mut trace_inspector = opts
        .trace
        .then(|| TraceInspector::new(initcode_map.clone()));
    let shared_labels = Arc::new(RwLock::new(local_state.cheatcodes.labels.clone()));
    if let Some(ref mut t) = trace_inspector {
        t.set_shared_labels(Arc::clone(&shared_labels));
    }

    let shared_labels = Arc::new(RwLock::new(local_state.cheatcodes.labels.clone()));
    let cheatcode_inspector = crate::chain::inspectors::cheatcode::CheatcodeInspector::from_state(
        local_state.cheatcodes.clone(),
    )
    .with_shared_labels(shared_labels);

    let mut inspector = InspectorTuple::new(
        coverage_inspector,
        MaybeTrace(trace_inspector),
        cheatcode_inspector,
    );

    // 3. Run each call in the sequence.
    let mut call_meta = Vec::with_capacity(calls.len());
    let mut all_ok = true;

    for (idx, call) in calls.iter().enumerate().take(config.max_sequence_calls) {
        local_state.advance_block(call.block_number_delay, call.block_timestamp_delay, idx);

        // Apply persistent prank to the top-level transaction caller.
        let mut clear_single_call_prank = false;
        if let Some(ref p) = inspector.2.state.prank.active {
            clear_single_call_prank = p.single_call;
        }
        let caller = inspector
            .2
            .state
            .prank
            .caller_for_top_level()
            .unwrap_or(config.caller);
        if clear_single_call_prank {
            inspector.2.state.prank.active = None;
        }

        let nonce = local_state.next_nonce();
        let mut ctx = Context::mainnet().with_db(local_state.db);
        ctx.cfg.disable_balance_check = true;
        ctx.block.number = U256::from(local_state.block_number);
        ctx.block.timestamp = U256::from(local_state.block_timestamp);

        // Apply non-timestamp/number block overrides from cheatcodes.
        // Timestamp and number are managed via local_state.block_timestamp /
        // block_number so advance_block delays compose correctly.
        let overrides = inspector.2.state.block_overrides();
        if let Some(fee) = overrides.basefee {
            ctx.block.basefee = fee;
        }
        if let Some(beneficiary) = overrides.beneficiary {
            ctx.block.beneficiary = beneficiary;
        }
        if let Some(prevrandao) = overrides.prevrandao {
            ctx.block.prevrandao = Some(prevrandao);
        }
        if let Some(chain_id) = overrides.chain_id {
            ctx.cfg.chain_id = chain_id;
        }

        let mut tx = TxEnv {
            caller,
            kind: TxKind::Call(contract_address),
            data: Bytes::from(call.encode()),
            gas_limit: config.gas_limit,
            nonce,
            ..Default::default()
        };
        tx.chain_id = Some(ctx.cfg.chain_id);
        tx.gas_price = ctx.block.basefee as u128;

        // Remember the pre-call block cheat state so we can restore on revert
        // and detect whether the block context was modified during this call.
        let prev_block = inspector.2.state.block;

        let mut evm = ctx.build_mainnet_with_inspector(inspector);
        let result = evm
            .inspect_tx_commit(tx)
            .map_err(|e| -> anyhow::Error { e.into() })?;

        // Re-extract inspector and db for the next iteration.
        inspector = evm.inspector;
        local_state.db = evm.ctx.journaled_state.database;

        call_meta.push(CallMeta {
            block_number: local_state.block_number,
            block_timestamp: local_state.block_timestamp,
        });

        if result.is_success() {
            // Sync block context into ChainState so the next call sees it,
            // but only if it was modified during this call (not carried over
            // from a previous call).
            if inspector.2.state.block.timestamp != prev_block.timestamp
                && let Some(ts) = inspector.2.state.block.timestamp
            {
                local_state.block_timestamp = u64::try_from(ts).unwrap_or(u64::MAX);
            }
            if inspector.2.state.block.number != prev_block.number
                && let Some(num) = inspector.2.state.block.number
            {
                local_state.block_number = u64::try_from(num).unwrap_or(u64::MAX);
            }
            // Sync remaining block overrides back to ChainState so that
            // property checks see fee, coinbase, prevrandao, and chain_id mutations.
            local_state.cheatcodes.block = inspector.2.state.block;
            trace!(idx, "call succeeded");
        } else {
            // Undo the block context so it does not leak into properties or
            // future calls (if we ever stop aborting on revert).
            inspector.2.state.block = prev_block;
            all_ok = false;
            trace!(idx, "call reverted, aborting sequence");
            break;
        }
    }

    // 4. Check properties (read-only calls against the final state).
    let property_results = check_properties(
        &mut local_state,
        contract_address,
        properties,
        contract_abi,
        config,
    )?;

    // 5. Assemble output from owned inspectors.
    let coverage = inspector.0.into_coverage();
    let trace = inspector.1.0.map(|t| t.into_trace_tree());

    Ok(ExecutionOutput {
        coverage,
        trace,
        call_meta,
        property_results,
        all_ok,
    })
}

fn check_properties(
    state: &mut ChainState,
    contract_address: revm::primitives::Address,
    properties: &[([u8; 4], String)],
    _contract_abi: &alloy_json_abi::JsonAbi,
    config: &ChainConfig,
) -> Result<Vec<PropertyResult>, ChainExecutionError> {
    let mut results = Vec::with_capacity(properties.len());

    for (selector, name) in properties {
        let db = std::mem::take(&mut state.db);
        let mut ctx = Context::mainnet().with_db(db);
        ctx.cfg.disable_balance_check = true;
        ctx.block.number = U256::from(state.block_number);
        ctx.block.timestamp = U256::from(state.block_timestamp);

        // Apply non-timestamp/number block overrides from cheatcodes.
        let overrides = state.cheatcodes.block_overrides();
        if let Some(fee) = overrides.basefee {
            ctx.block.basefee = fee;
        }
        if let Some(beneficiary) = overrides.beneficiary {
            ctx.block.beneficiary = beneficiary;
        }
        if let Some(prevrandao) = overrides.prevrandao {
            ctx.block.prevrandao = Some(prevrandao);
        }
        if let Some(chain_id) = overrides.chain_id {
            ctx.cfg.chain_id = chain_id;
        }

        let mut tx = TxEnv {
            caller: config.caller,
            kind: TxKind::Call(contract_address),
            data: Bytes::copy_from_slice(selector),
            gas_limit: config.gas_limit,
            nonce: state.next_nonce(),
            ..Default::default()
        };
        tx.chain_id = Some(ctx.cfg.chain_id);
        tx.gas_price = ctx.block.basefee as u128;
        let mut evm = ctx.build_mainnet_with_inspector(NoOpInspector);
        let result = evm.inspect_tx_commit(tx);
        state.db = evm.ctx.journaled_state.database;
        let result = result.map_err(|e| -> anyhow::Error { e.into() })?;

        let passed = if result.is_success() {
            let out = result.output();
            if let Some(output) = out
                && output.len() == 32
                && output[31] == 1
            {
                true
            } else {
                false
            }
        } else {
            false
        };

        results.push(PropertyResult {
            name: name.into(),
            selector: *selector,
            passed,
        });

        if passed {
            trace!(property = %name, "property returned true");
        }
    }

    Ok(results)
}
