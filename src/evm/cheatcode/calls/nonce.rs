//! `setNonce` / `getNonce` cheatcodes.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Address, U256},
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub fn set_nonce<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    addr: Address,
    nonce: u64,
    gas_limit: u64,
    ctx: &mut CTX,
    _state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let current = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    if nonce < current {
        return Some(util::revert(
            &format!("new nonce ({nonce}) must be >= current nonce ({current})"),
            gas_limit,
        ));
    }
    let mut acc = ctx
        .journal_mut()
        .load_account_mut(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    acc.data.set_nonce(nonce);
    Some(util::success(gas_limit))
}

pub fn get_nonce<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    addr: Address,
    gas_limit: u64,
    ctx: &mut CTX,
    _state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let nonce = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    Some(util::success_u256(U256::from(nonce), gas_limit))
}
