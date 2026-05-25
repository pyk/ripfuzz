//! `getCode` cheatcode - read compiled bytecode by contract name.

use crate::evm::cheatcode::{state::ExecutionState, util};

pub fn handle(
    name: &str,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let initcode = state.compiled_contracts.get(name)?;
    if initcode.is_empty() {
        return Some(util::revert(
            &format!("getCode: bytecode is empty: {name}"),
            gas_limit,
        ));
    }
    let encoded = alloy_dyn_abi::DynSolValue::Bytes(initcode.to_vec()).abi_encode();
    Some(util::success_bytes(encoded, gas_limit))
}
