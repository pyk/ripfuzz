//! Sequence execution engine for the fuzzer.

use alloy_primitives::{Address, Selector, U256};
use anyhow::{Context, Result};
use revm::{
    context::TxEnv,
    primitives::{Bytes, TxKind},
};

use crate::evm;
use crate::evm::cheatcode;
use crate::evm::coverage;
use crate::evm::coverage::map::LocalCoverage;
use crate::fuzzer::corpus::Call;

/// Result of executing a single call sequence.
#[derive(Debug, Clone, Default)]
pub struct ExecutionOutcome {
    pub coverage: LocalCoverage,
    pub all_ok: bool,
    pub total_calls: u64,
    pub total_gas: u64,
    pub crash: Option<CrashInfo>,
}

/// Details about an assert-panic crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashInfo {
    pub name: String,
    pub selector: Selector,
}

/// Solidity `Panic(uint256)` selector: keccak256("Panic(uint256)")[:4]
const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];

/// Detect a Solidity `assert` failure (`Panic(0x01)`) in revert output.
pub fn is_assert_failure(output: &Bytes) -> bool {
    output.len() >= 36 && output[..4] == PANIC_SELECTOR && output[35] == 0x01
}

/// Execute a call sequence against a cloned chain state.
///
/// The chain is cloned so the base state is never mutated. Block delays
/// are applied between calls and invariant functions are checked after
/// the target sequence completes.
pub fn execute_sequence(
    base_chain: &evm::Chain,
    contract: &evm::Contract,
    deployed_address: Address,
    caller: Address,
    calls: &[Call],
) -> Result<ExecutionOutcome> {
    let mut chain = base_chain.clone();
    let mut inspector = (
        cheatcode::Inspector::from_state(chain.cheatcode_state.clone()),
        coverage::Inspector::new(),
    );

    let mut total_calls = 0u64;
    let mut total_gas = 0u64;
    let mut all_ok = true;
    let mut crash = None;

    for (idx, call) in calls.iter().enumerate() {
        let current_number = u64::try_from(chain.block_env().number).unwrap_or(u64::MAX);
        let current_timestamp = u64::try_from(chain.block_env().timestamp).unwrap_or(u64::MAX);
        // Medusa-style: each subsequent call advances 1 block and 1
        // timestamp so every call has a unique block context.
        let new_number = if idx > 0 {
            current_number.saturating_add(1)
        } else {
            current_number
        };
        let new_timestamp = if idx > 0 {
            current_timestamp.saturating_add(1)
        } else {
            current_timestamp
        };
        chain.block_env_mut().number = U256::from(new_number);
        chain.block_env_mut().timestamp = U256::from(new_timestamp);

        let tx_origin = inspector.0.state.prank.origin_for_top_level(caller);

        let tx = TxEnv {
            caller: tx_origin,
            kind: TxKind::Call(deployed_address),
            data: call.calldata(),
            gas_limit: u64::MAX,
            value: U256::ZERO,
            ..Default::default()
        };

        let (result, (cheatcode_insp, coverage_insp)) = chain
            .inspect(tx, inspector)
            .context("revm transaction failed")?;
        inspector = (cheatcode_insp, coverage_insp);

        total_calls += 1;
        let gas_used = result.gas_used;
        total_gas += gas_used;

        let success = result.success;

        if !success {
            if let Some(ref output) = result.output
                && is_assert_failure(output)
            {
                crash = Some(CrashInfo {
                    // checkrs: allow(clone_in_loops)
                    name: call.function.name.clone(),
                    selector: call.selector(),
                });
            }
            all_ok = false;
            break;
        }
    }

    // Check invariants
    if all_ok {
        for inv in &contract.invariant_functions {
            let current_number = u64::try_from(chain.block_env().number).unwrap_or(u64::MAX);
            let current_timestamp = u64::try_from(chain.block_env().timestamp).unwrap_or(u64::MAX);
            let new_number = current_number.saturating_add(1);
            let new_timestamp = current_timestamp.saturating_add(1);
            chain.block_env_mut().number = U256::from(new_number);
            chain.block_env_mut().timestamp = U256::from(new_timestamp);

            let tx = TxEnv {
                caller,
                kind: TxKind::Call(deployed_address),
                data: Bytes::from(inv.selector().as_slice().to_vec()),
                gas_limit: u64::MAX,
                value: U256::ZERO,
                ..Default::default()
            };

            let (result, (cheatcode_insp, coverage_insp)) = chain
                .inspect(tx, inspector)
                .context("revm transaction failed")?;
            inspector = (cheatcode_insp, coverage_insp);

            total_calls += 1;
            let gas_used = result.gas_used;
            total_gas += gas_used;

            let success = result.success;

            if !success {
                if let Some(ref output) = result.output
                    && is_assert_failure(output)
                {
                    crash = Some(CrashInfo {
                        name: inv.name.to_owned(),
                        selector: inv.selector(),
                    });
                }
                all_ok = false;
                break;
            }
        }
    }

    let coverage = {
        let (_, coverage_inspector) = inspector;
        coverage_inspector.into_coverage()
    };

    Ok(ExecutionOutcome {
        coverage,
        all_ok,
        total_calls,
        total_gas,
        crash,
    })
}
