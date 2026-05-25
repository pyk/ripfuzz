//! `setNonce` / `getNonce` cheatcodes.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Address, U256},
};

use crate::evm::cheatcode::outcome;

pub fn set_nonce<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
    nonce: u64,
) -> Option<revm::interpreter::CallOutcome> {
    let current = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    if nonce < current {
        return Some(outcome::revert(&format!(
            "new nonce ({nonce}) must be >= current nonce ({current})"
        )));
    }
    let mut acc = ctx
        .journal_mut()
        .load_account_mut(addr)
        .map_err(|_| "account load failed")
        .ok()?;
    acc.data.set_nonce(nonce);
    Some(outcome::success())
}

pub fn get_nonce<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    ctx: &mut CTX,
    addr: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let nonce = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    Some(outcome::success_u256(U256::from(nonce)))
}
