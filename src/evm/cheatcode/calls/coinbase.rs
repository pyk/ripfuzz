//! `coinbase` cheatcode - set and persist `block.coinbase`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Address,
};

use crate::evm::cheatcode::{outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    addr: Address,

    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.beneficiary = addr;
    ctx.set_block(block);
    state.block.beneficiary = Some(addr);
    Some(outcome::success())
}
