//! `difficulty` cheatcode - set and persist `block.difficulty` / `prevrandao`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::U256,
};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    value: U256,

    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let bytes: [u8; 32] = value.to_be_bytes();
    let mut block = ctx.block().clone();
    block.difficulty = value;
    block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    ctx.set_block(block);
    state.block.difficulty = Some(value);
    state.block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    Some(outcome::success())
}
