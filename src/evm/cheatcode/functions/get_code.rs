//! `getCode` cheatcode - read compiled bytecode by contract name.

use revm::primitives::Bytes;

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const SELECTOR: [u8; 4] = [0x8d, 0x1c, 0xc9, 0x25];

pub fn handle(
    input: &Bytes,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let name = util::decode_string(input)?;
    let initcode = state.compiled_contracts.get(&name)?;
    if initcode.is_empty() {
        return Some(util::revert(
            &format!("getCode: bytecode is empty: {name}"),
            gas_limit,
        ));
    }
    let encoded = alloy_dyn_abi::DynSolValue::Bytes(initcode.to_vec()).abi_encode();
    Some(util::success_bytes(encoded, gas_limit))
}
