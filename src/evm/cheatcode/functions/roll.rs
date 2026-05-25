//! `roll` cheatcode - set and persist `block.number`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Bytes,
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const SELECTOR: [u8; 4] = [0x1f, 0x7b, 0x4f, 0x30];

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let value = util::decode_u256(input)?;
    let mut block = ctx.block().clone();
    block.number = value;
    ctx.set_block(block);
    state.block.number = Some(value);
    Some(util::success(gas_limit))
}
