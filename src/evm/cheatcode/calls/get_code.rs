//! `getCode` cheatcode - read compiled bytecode by contract name.

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle(
    name: &str,
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let initcode = state.compiled_contracts.get(name)?;
    if initcode.is_empty() {
        return Some(outcome::revert(
            &format!("getCode: bytecode is empty: {name}"),
            gas_limit,
        ));
    }
    let encoded = alloy_dyn_abi::DynSolValue::Bytes(initcode.to_vec()).abi_encode();
    Some(outcome::success_bytes(encoded, gas_limit))
}
