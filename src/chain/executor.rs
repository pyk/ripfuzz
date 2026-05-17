//! Sequence execution: runs a call sequence against a cloned chain state.

use std::collections::HashMap;

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
    inspectors::{CompositeInspector, coverage::CoverageInspector, trace::TraceInspector},
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
    let trace_inspector = opts
        .trace
        .then(|| TraceInspector::new(initcode_map.clone()));

    let mut inspector = CompositeInspector::new(coverage_inspector, trace_inspector)
        .with_cheatcodes(local_state.cheatcodes.clone());

    // 3. Run each call in the sequence.
    let mut call_meta = Vec::with_capacity(calls.len());
    let mut all_ok = true;

    for (idx, call) in calls.iter().enumerate().take(config.max_sequence_calls) {
        local_state.advance_block(call.block_number_delay, call.block_timestamp_delay, idx);

        // Apply persistent prank to the top-level transaction caller.
        // Internal CALLs are handled by CheatcodeInspector::call; here we
        // handle the outer transaction frame that revm does not route through
        // the call hook.
        let mut caller = config.caller;
        let mut clear_single_call_prank = false;
        if let Some(ref cheatcodes) = inspector.cheatcodes {
            match (
                cheatcodes.state.start_prank.as_ref(),
                cheatcodes.state.prank.as_ref(),
            ) {
                (Some(start_prank), _) => caller = start_prank.caller,
                (None, Some(prank)) => {
                    caller = prank.caller;
                    clear_single_call_prank = prank.single_call;
                }
                (None, None) => {}
            }
        }
        if clear_single_call_prank && let Some(ref mut cheatcodes) = inspector.cheatcodes {
            cheatcodes.state.prank = None;
        }

        let nonce = local_state.next_nonce();
        let mut ctx = Context::mainnet().with_db(local_state.db);
        ctx.block.number = U256::from(local_state.block_number);
        ctx.block.timestamp = U256::from(local_state.block_timestamp);
        if let Some(fee) = local_state.cheatcodes.fee {
            ctx.block.basefee = u64::try_from(fee).unwrap_or(0);
        }
        if let Some(coinbase) = local_state.cheatcodes.coinbase {
            ctx.block.beneficiary = coinbase;
        }
        if let Some(prevrandao) = local_state.cheatcodes.prevrandao {
            ctx.block.prevrandao = Some(revm::primitives::FixedBytes::from(prevrandao));
        }
        if let Some(chain_id) = local_state.cheatcodes.chain_id {
            ctx.cfg.chain_id = chain_id.as_limbs()[0];
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

        if !result.is_success() {
            all_ok = false;
            trace!(idx, "call reverted, aborting sequence");
            break;
        }
        trace!(idx, "call succeeded");
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
    let coverage = inspector.coverage.into_coverage();
    let trace = inspector.trace.map(|t| t.into_trace_tree());

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
        // Property functions are view/pure — revm reverts on any SSTORE,
        // so the DB is functionally unchanged.  We must move the DB into
        // revm's Context (it does not support borrowing) and move it back
        // out afterward.  `std::mem::take` is a zero-cost placeholder
        // because InMemoryDB::default() is cheap; the real DB is restored
        // before the `?` so error paths never lose it.
        let db = std::mem::take(&mut state.db);
        let mut ctx = Context::mainnet().with_db(db);
        ctx.block.number = U256::from(state.block_number);
        ctx.block.timestamp = U256::from(state.block_timestamp);
        if let Some(fee) = state.cheatcodes.fee {
            ctx.block.basefee = u64::try_from(fee).unwrap_or(0);
        }
        if let Some(coinbase) = state.cheatcodes.coinbase {
            ctx.block.beneficiary = coinbase;
        }
        if let Some(prevrandao) = state.cheatcodes.prevrandao {
            ctx.block.prevrandao = Some(revm::primitives::FixedBytes::from(prevrandao));
        }
        if let Some(chain_id) = state.cheatcodes.chain_id {
            ctx.cfg.chain_id = chain_id.as_limbs()[0];
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
