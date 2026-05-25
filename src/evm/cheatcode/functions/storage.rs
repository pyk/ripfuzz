//! `store` / `load` cheatcodes.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Bytes, U256},
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const STORE_SELECTOR: [u8; 4] = [0x66, 0x7f, 0x9d, 0x70];
pub const LOAD_SELECTOR: [u8; 4] = [0x70, 0xca, 0x10, 0xbb];

pub fn store<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    _state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let (addr, slot, value) = util::decode_address_bytes32_bytes32(input)?;
    if ctx.journal().precompile_addresses().contains(&addr) {
        return Some(util::revert("store: cannot write to precompile", gas_limit));
    }
    ctx.journal_mut()
        .load_account(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    ctx.journal_mut()
        .sstore(addr, U256::from_be_bytes(slot), U256::from_be_bytes(value))
        .map_err(|e| format!("failed to store storage slot: {e:?}"))
        .ok()?;
    Some(util::success(gas_limit))
}

pub fn load<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    _state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let (addr, slot) = util::decode_address_bytes32(input)?;
    if ctx.journal().precompile_addresses().contains(&addr) {
        return Some(util::revert("load: cannot read from precompile", gas_limit));
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
    Some(util::success_bytes(value.to_be_bytes_vec(), gas_limit))
}
