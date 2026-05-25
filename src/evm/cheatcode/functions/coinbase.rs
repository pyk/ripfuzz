//! `coinbase` cheatcode - set and persist `block.coinbase`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Bytes,
};

use crate::evm::cheatcode::{state::ExecutionState, util};

pub const SELECTOR: [u8; 4] = [0xff, 0x48, 0x3c, 0x54];

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv>>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let addr = util::decode_address(input)?;
    let mut block = ctx.block().clone();
    block.beneficiary = addr;
    ctx.set_block(block);
    state.block.beneficiary = Some(addr);
    Some(util::success(gas_limit))
}
