//! `prevrandao` cheatcode - set and persist `block.prevrandao`.

use revm::{context::BlockEnv, context::ContextSetters, context_interface::ContextTr};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    bytes: [u8; 32],
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    ctx.set_block(block);
    state.block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    Some(outcome::success(gas_limit))
}
