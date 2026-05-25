//! `prevrandao` cheatcode - set and persist `block.prevrandao`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Bytes,
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const SELECTOR: [u8; 4] = [0x3b, 0x92, 0x55, 0x49];

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let (_, bytes) = util::decode_address_bytes32(input)?;
    let mut block = ctx.block().clone();
    block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    ctx.set_block(block);
    state.block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    Some(util::success(gas_limit))
}
