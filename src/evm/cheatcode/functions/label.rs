//! `label` / `getLabel` cheatcodes.

use alloy_dyn_abi::DynSolValue;
use revm::primitives::Bytes;

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const LABEL_SELECTOR: [u8; 4] = [0xc6, 0x57, 0xc7, 0x18];
pub const GET_LABEL_SELECTOR: [u8; 4] = [0x28, 0xa2, 0x49, 0xb0];

pub fn label(
    input: &Bytes,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let (addr, name) = util::decode_address_string(input)?;
    state.labels.insert(addr, name);
    Some(util::success(gas_limit))
}

pub fn get_label(
    input: &Bytes,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let addr = util::decode_address(input)?;
    let name = state.labels.get(&addr).cloned().unwrap_or_default();
    let encoded = DynSolValue::String(name).abi_encode();
    Some(util::success_bytes(encoded, gas_limit))
}
