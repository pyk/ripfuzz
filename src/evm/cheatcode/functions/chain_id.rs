//! `chainId` cheatcode - set and persist `chain_id`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::Bytes,
};

use crate::evm::cheatcode::{inspector::CfgMut, state::ExecutionState, util};

pub const SELECTOR: [u8; 4] = [0x40, 0x49, 0xdd, 0xd2];

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv> + CfgMut>(
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let value = util::decode_u256(input)?;
    let id = u64::try_from(value).unwrap_or(u64::MAX);
    ctx.set_chain_id(id);
    state.block.chain_id = Some(value);
    Some(util::success(gas_limit))
}
