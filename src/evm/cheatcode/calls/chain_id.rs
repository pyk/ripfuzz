//! `chainId` cheatcode - set and persist `chain_id`.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr, primitives::U256,
};

use crate::evm::cheatcode::{inspector::CfgMut, outcome, state::ExecutionState};

pub fn handle<CTX: ContextTr + ContextSetters<Block = BlockEnv> + CfgMut>(
    value: U256,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let id = u64::try_from(value).unwrap_or(u64::MAX);
    ctx.set_chain_id(id);
    state.block.chain_id = Some(value);
    Some(outcome::success(gas_limit))
}
