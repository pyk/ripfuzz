//! Sequence execution: runs a call sequence against a cloned chain state.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use alloy_json_abi::JsonAbi;
use revm::{
    MainBuilder, MainContext,
    context::{Context, TxEnv},
    inspector::InspectCommitEvm,
    primitives::{Bytes, TxKind, U256},
};

use anyhow::Result;
use tracing::trace;

use crate::chain::{
    ChainConfig,
    base_state::BaseState,
    error::ChainExecutionError,
    init::decode_solidity_error,
    inspectors::{InspectorTuple, MaybeTrace, trace::TraceInspector},
    output::{CrashInfo, ExecutionOutput},
};
use crate::corpus::{Call, CallMeta};
use crate::evm::{cheatcode, coverage};

/// Something that can execute a sequence of calls and return the outcome.
pub trait SequenceExecutor: Send + Sync {
    fn execute(&self, calls: &[Call]) -> Result<ExecutionOutput>;
}

/// Options controlling expensive execution features.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutionOptions {
    /// Enable call-trace collection.
    pub trace: bool,
}

/// Solidity `Panic(uint256)` selector: keccak256("Panic(uint256)")[:4]
const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];

/// Detect a Solidity `assert` failure (`Panic(0x01)`) in revert output.
fn is_assert_failure(output: &Bytes) -> bool {
    output.len() >= 36 && output[..4] == PANIC_SELECTOR && output[35] == 0x01
}

/// Execute a call sequence (plus appended invariant calls) against a cloned post-setup state.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    state: &BaseState,
    contract_address: revm::primitives::Address,
    invariants: &[([u8; 4], String)],
    contract_abi: &alloy_json_abi::JsonAbi,
    config: &ChainConfig,
    initcode_map: &HashMap<Bytes, (String, JsonAbi)>,
    calls: &[Call],
    opts: ExecutionOptions,
) -> Result<ExecutionOutput, ChainExecutionError> {
    // 1. Clone the immutable post-setup state.
    let mut local_state = state.clone();

    // 2. Build inspectors (owned, no external mutable references).
    let coverage_inspector = coverage::Inspector::new();
    let mut trace_inspector = opts
        .trace
        .then(|| TraceInspector::new(initcode_map.clone()));
    let shared_labels = Arc::new(RwLock::new(local_state.labels.clone()));
    if let Some(ref mut t) = trace_inspector {
        t.set_shared_labels(Arc::clone(&shared_labels));
    }

    let exec_state = crate::evm::cheatcode::ExecutionState {
        project_root: local_state.project_root.clone(),
        ffi_enabled: local_state.ffi_enabled,
        compiled_contracts: local_state.compiled_contracts.clone(),
        labels: local_state.labels.clone(),
        prank: local_state.prank.clone(),
        block: local_state.block_overrides,
    };
    let cheatcode_inspector =
        cheatcode::Inspector::from_state(exec_state).with_shared_labels(shared_labels);

    let inspector = InspectorTuple::new(
        coverage_inspector,
        MaybeTrace(trace_inspector),
        cheatcode_inspector,
    );

    // 2.5 Build the EVM once and reuse it across all calls in the sequence.
    let db = std::mem::take(&mut local_state.db);
    let mut ctx = Context::mainnet().with_db(db);
    ctx.cfg.disable_balance_check = true;
    ctx.cfg.tx_gas_limit_cap = Some(u64::MAX);
    ctx.block.number = U256::from(local_state.block_number);
    ctx.block.timestamp = U256::from(local_state.block_timestamp);

    let overrides = inspector.2.state.block;
    if let Some(fee) = overrides.basefee {
        ctx.block.basefee = u64::try_from(fee).unwrap_or_default();
    }
    if let Some(beneficiary) = overrides.beneficiary {
        ctx.block.beneficiary = beneficiary;
    }
    if let Some(prevrandao) = overrides.prevrandao {
        ctx.block.prevrandao = Some(prevrandao);
    }
    if let Some(chain_id) = overrides.chain_id {
        ctx.cfg.chain_id = u64::try_from(chain_id).unwrap_or_default();
    }

    let mut evm = ctx.build_mainnet_with_inspector(inspector);

    // 3. Build combined call sequence: fuzzed calls + invariant calls.
    let mut sequence = calls.to_vec();
    for (selector, _name) in invariants {
        sequence.push(Call {
            selector: *selector,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        });
    }

    // 4. Run each call in the combined sequence.
    let mut call_meta = Vec::with_capacity(sequence.len());
    let mut all_ok = true;
    let mut total_calls = 0u64;
    let mut total_gas = 0u64;
    let mut crash: Option<CrashInfo> = None;

    for (idx, call) in sequence.iter().enumerate().take(config.max_sequence_calls) {
        local_state.advance_block(call.block_number_delay, call.block_timestamp_delay, idx);

        evm.ctx.block.number = U256::from(local_state.block_number);
        evm.ctx.block.timestamp = U256::from(local_state.block_timestamp);

        let overrides = evm.inspector.2.state.block;
        if let Some(fee) = overrides.basefee {
            evm.ctx.block.basefee = u64::try_from(fee).unwrap_or_default();
        }
        if let Some(beneficiary) = overrides.beneficiary {
            evm.ctx.block.beneficiary = beneficiary;
        }
        if let Some(prevrandao) = overrides.prevrandao {
            evm.ctx.block.prevrandao = Some(prevrandao);
        }
        if let Some(chain_id) = overrides.chain_id {
            evm.ctx.cfg.chain_id = u64::try_from(chain_id).unwrap_or_default();
        }

        let tx_origin = evm
            .inspector
            .2
            .state
            .prank
            .origin_for_top_level(config.caller);

        let nonce = local_state.next_nonce();
        let mut tx = TxEnv {
            caller: tx_origin,
            kind: TxKind::Call(contract_address),
            data: Bytes::from(call.encode()),
            gas_limit: u64::MAX,
            nonce,
            ..Default::default()
        };
        tx.chain_id = Some(evm.ctx.cfg.chain_id);
        tx.gas_price = evm.ctx.block.basefee as u128;

        let prev_block = evm.inspector.2.state.block;

        let result = evm
            .inspect_tx_commit(tx)
            .map_err(|e| -> anyhow::Error { e.into() })?;

        total_calls += 1;
        let gas_used = result.tx_gas_used();
        total_gas += gas_used;

        let success = result.is_success();
        let reason = if !success {
            match &result {
                revm::context::result::ExecutionResult::Halt { reason, .. } => {
                    Some(format!("halted: {reason}"))
                }
                revm::context::result::ExecutionResult::Revert { output, .. } => {
                    decode_solidity_error(output)
                        .map(|r| format!("reverted: {r}"))
                        .or_else(|| Some("reverted".into()))
                }
                _ => Some("failed".into()),
            }
        } else {
            None
        };

        call_meta.push(CallMeta {
            block_number: local_state.block_number,
            block_timestamp: local_state.block_timestamp,
            gas_used,
            success,
            reason,
        });

        if result.is_success() {
            let inspector = &mut evm.inspector;
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
            trace!(idx, "call succeeded");
        } else {
            let inspector = &mut evm.inspector;
            inspector.2.state.block = prev_block;
            if let Some(output) = result.output()
                && is_assert_failure(output)
            {
                let name = contract_abi
                    .functions()
                    .find(|f| f.selector().as_slice() == call.selector)
                    .map(|f| f.name.to_owned())
                    .unwrap_or_else(|| format!("0x{}", hex::encode(call.selector)));
                crash = Some(CrashInfo {
                    name,
                    selector: call.selector,
                });
            }
            all_ok = false;
            trace!(idx, "call failed, aborting sequence");
            break;
        }
    }

    // Extract inspector and db.
    let inspector = evm.inspector;
    local_state.db = evm.ctx.journaled_state.database;

    // 5. Assemble output from owned inspectors.
    let coverage = inspector.0.into_coverage();
    let trace = inspector.1.0.map(|t| t.into_trace_tree());

    Ok(ExecutionOutput {
        coverage,
        trace,
        call_meta,
        all_ok,
        total_calls,
        total_gas,
        crash,
    })
}
