//! `etch` cheatcode - set contract bytecode at an address.

use revm::{
    bytecode::Bytecode,
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr},
    primitives::{Address, Bytes},
};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    addr: Address,
    code: Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    _state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    if ctx.journal().precompile_addresses().contains(&addr) {
        return Some(outcome::revert("cannot etch precompile address", gas_limit));
    }
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    let bytecode = Bytecode::new_raw_checked(code)
        .map_err(|e| format!("failed to create bytecode: {e}"))
        .ok()?;
    ctx.journal_mut().set_code(addr, bytecode);
    Some(outcome::success(gas_limit))
}
