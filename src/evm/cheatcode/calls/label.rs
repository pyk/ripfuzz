//! `label` / `getLabel` cheatcodes.

use alloy_dyn_abi::DynSolValue;
use revm::primitives::Address;

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn label(
    addr: Address,
    name: &str,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    state.labels.insert(addr, name.into());
    Some(outcome::success(gas_limit))
}

pub fn get_label(
    addr: Address,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let name = state.labels.get(&addr).cloned().unwrap_or_default();
    let encoded = DynSolValue::String(name).abi_encode();
    Some(outcome::success_bytes(encoded, gas_limit))
}
