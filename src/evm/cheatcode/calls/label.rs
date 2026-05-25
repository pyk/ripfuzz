//! `label` / `getLabel` cheatcodes.

use alloy_dyn_abi::DynSolValue;
use revm::primitives::Address;

use crate::evm::cheatcode::{state::ExecutionState, util};

pub fn label(
    addr: Address,
    name: &str,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    state.labels.insert(addr, name.into());
    Some(util::success(gas_limit))
}

pub fn get_label(
    addr: Address,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let name = state.labels.get(&addr).cloned().unwrap_or_default();
    let encoded = DynSolValue::String(name).abi_encode();
    Some(util::success_bytes(encoded, gas_limit))
}
