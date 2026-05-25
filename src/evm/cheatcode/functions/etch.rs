//! `etch` cheatcode - set contract bytecode at an address.

use revm::{
    bytecode::Bytecode,
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr},
    primitives::Bytes,
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const SELECTOR: [u8; 4] = [0xb4, 0xd6, 0xc7, 0x82];

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    _state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let (addr, code) = util::decode_address_bytes(input)?;
    if ctx.journal().precompile_addresses().contains(&addr) {
        return Some(util::revert("cannot etch precompile address", gas_limit));
    }
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    let bytecode = Bytecode::new_raw_checked(code)
        .map_err(|e| format!("failed to create bytecode: {e}"))
        .ok()?;
    ctx.journal_mut().set_code(addr, bytecode);
    Some(util::success(gas_limit))
}
