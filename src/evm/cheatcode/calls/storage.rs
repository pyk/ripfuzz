//! `store` / `load` cheatcodes.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Address, U256},
};

use crate::evm::cheatcode::outcome;

pub fn store<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    slot: [u8; 32],
    value: [u8; 32],
) -> Option<revm::interpreter::CallOutcome> {
    if ctx.journal().precompile_addresses().contains(&addr) {
        return Some(outcome::revert("store: cannot write to precompile"));
    }
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    ctx.journal_mut()
        .sstore(addr, U256::from_be_bytes(slot), U256::from_be_bytes(value))
        .map_err(|e| format!("failed to store storage slot: {e:?}"))
        .ok()?;
    Some(outcome::success())
}

pub fn load<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    slot: [u8; 32],
) -> Option<revm::interpreter::CallOutcome> {
    if ctx.journal().precompile_addresses().contains(&addr) {
        return Some(outcome::revert("load: cannot read from precompile"));
    }
    let value = match ctx.journal_mut().load_account_mut(addr) {
        Ok(mut s) => s
            .data
            .sload(U256::from_be_bytes(slot), false)
            .ok()
            .map(|r| r.data.present_value)
            .unwrap_or(U256::ZERO),
        Err(_) => U256::ZERO,
    };
    Some(outcome::success_bytes(value.to_be_bytes_vec()))
}
