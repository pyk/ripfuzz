//! `difficulty` cheatcode - set and persist `block.difficulty` / `prevrandao`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Bytes,
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const SELECTOR: [u8; 4] = [0x46, 0xcc, 0x92, 0xd9];

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let value = util::decode_u256(input)?;
    let bytes: [u8; 32] = value.to_be_bytes();
    let mut block = ctx.block().clone();
    block.difficulty = value;
    block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    ctx.set_block(block);
    state.block.difficulty = Some(value);
    state.block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    Some(util::success(gas_limit))
}
