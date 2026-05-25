//! `warp` cheatcode - set and persist `block.timestamp`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::U256,
};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    value: U256,

    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.timestamp = value;
    ctx.set_block(block);
    state.block.timestamp = Some(value);
    Some(outcome::success())
}
