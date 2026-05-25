//! `warp` cheatcode - set and persist `block.timestamp`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Bytes,
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const SELECTOR: [u8; 4] = [0xe5, 0xd6, 0xbf, 0x02];

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let value = util::decode_u256(input)?;
    let mut block = ctx.block().clone();
    block.timestamp = value;
    ctx.set_block(block);
    state.block.timestamp = Some(value);
    Some(util::success(gas_limit))
}
