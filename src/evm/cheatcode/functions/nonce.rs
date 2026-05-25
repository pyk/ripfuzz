//! `setNonce` / `getNonce` cheatcodes.

use revm::{
    context::BlockEnv,
    context::ContextSetters,
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    primitives::{Bytes, U256},
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const SET_NONCE_SELECTOR: [u8; 4] = [0xf8, 0xe1, 0x8b, 0x57];
pub const GET_NONCE_SELECTOR: [u8; 4] = [0x2d, 0x03, 0x35, 0xab];

pub fn set_nonce<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    _state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let (addr, value) = util::decode_address_u256(input)?;
    let nonce = u64::try_from(value).unwrap_or(0);
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
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    _state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let addr = util::decode_address(input)?;
    let nonce = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    Some(util::success_u256(U256::from(nonce), gas_limit))
}
