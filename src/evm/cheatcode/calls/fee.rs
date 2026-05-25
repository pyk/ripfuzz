//! `fee` cheatcode - set and persist `block.basefee`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::U256,
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    value: U256,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.basefee = u64::try_from(value).unwrap_or(0);
    ctx.set_block(block);
    state.block.basefee = Some(value);
    Some(util::success(gas_limit))
}
