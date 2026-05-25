//! `coinbase` cheatcode - set and persist `block.coinbase`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Address,
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    addr: Address,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let mut block = ctx.block().clone();
    block.beneficiary = addr;
    ctx.set_block(block);
    state.block.beneficiary = Some(addr);
    Some(util::success(gas_limit))
}
