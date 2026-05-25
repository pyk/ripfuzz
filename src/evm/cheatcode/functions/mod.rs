//! Cheatcode handlers - one submodule per Foundry-compatible cheatcode.

use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr,
    interpreter::CallOutcome, primitives::Bytes,
};

use crate::evm::cheatcode::state::ExecutionState;

pub mod addr;
pub mod chain_id;
pub mod coinbase;
pub mod deal;
pub mod difficulty;
pub mod etch;
pub mod fee;
pub mod ffi;
pub mod get_code;
pub mod label;
pub mod nonce;
pub mod parse;
pub mod prank;
pub mod prevrandao;
pub mod roll;
pub mod sign;
pub mod storage;
pub mod to_string;
pub mod warp;

/// Dispatch a cheatcode selector to its handler.
pub fn dispatch<CTX>(
    selector: [u8; 4],
    input: &Bytes,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<CallOutcome>
where
    CTX: ContextTr + ContextSetters<Block = BlockEnv> + crate::evm::cheatcode::inspector::CfgMut,
{
    match selector {
        // Block
        warp::SELECTOR => warp::handle(input, gas_limit, ctx, state),
        roll::SELECTOR => roll::handle(input, gas_limit, ctx, state),
        fee::SELECTOR => fee::handle(input, gas_limit, ctx, state),
        coinbase::SELECTOR => coinbase::handle(input, gas_limit, ctx, state),
        prevrandao::SELECTOR => prevrandao::handle(input, gas_limit, ctx, state),
        difficulty::SELECTOR => difficulty::handle(input, gas_limit, ctx, state),
        chain_id::SELECTOR => chain_id::handle(input, gas_limit, ctx, state),

        // Account
        deal::SELECTOR => deal::handle(input, gas_limit, ctx, state),
        etch::SELECTOR => etch::handle(input, gas_limit, ctx, state),
        nonce::SET_NONCE_SELECTOR => nonce::set_nonce(input, gas_limit, ctx, state),
        nonce::GET_NONCE_SELECTOR => nonce::get_nonce(input, gas_limit, ctx, state),
        storage::STORE_SELECTOR => storage::store(input, gas_limit, ctx, state),
        storage::LOAD_SELECTOR => storage::load(input, gas_limit, ctx, state),

        // Prank
        prank::PRANK_SELECTOR => prank::prank(input, gas_limit, ctx, state),
        prank::PRANK_ORIGIN_SELECTOR => prank::prank_origin(input, gas_limit, ctx, state),
        prank::START_PRANK_SELECTOR => prank::start_prank(input, gas_limit, ctx, state),
        prank::START_PRANK_ORIGIN_SELECTOR => {
            prank::start_prank_origin(input, gas_limit, ctx, state)
        }
        prank::STOP_PRANK => prank::stop_prank(gas_limit, state),

        // Label
        label::LABEL_SELECTOR => label::label(input, gas_limit, state),
        label::GET_LABEL_SELECTOR => label::get_label(input, gas_limit, state),

        // Conversion
        to_string::TO_STRING_ADDRESS_SELECTOR => to_string::to_string_address(input, gas_limit),
        to_string::TO_STRING_BOOL_SELECTOR => to_string::to_string_bool(input, gas_limit),
        to_string::TO_STRING_UINT_SELECTOR => to_string::to_string_uint(input, gas_limit),
        to_string::TO_STRING_INT_SELECTOR => to_string::to_string_int(input, gas_limit),
        to_string::TO_STRING_BYTES32_SELECTOR => to_string::to_string_bytes32(input, gas_limit),
        to_string::TO_STRING_BYTES_SELECTOR => to_string::to_string_bytes(input, gas_limit),
        parse::PARSE_UINT_SELECTOR => parse::parse_uint(input, gas_limit),
        parse::PARSE_INT_SELECTOR => parse::parse_int(input, gas_limit),
        parse::PARSE_BOOL_SELECTOR => parse::parse_bool(input, gas_limit),
        parse::PARSE_ADDRESS_SELECTOR => parse::parse_address(input, gas_limit),
        parse::PARSE_BYTES_SELECTOR => parse::parse_bytes(input, gas_limit),
        parse::PARSE_BYTES32_SELECTOR => parse::parse_bytes32(input, gas_limit),

        // Code / wallet / ffi
        get_code::SELECTOR => get_code::handle(input, gas_limit, state),
        addr::SELECTOR => addr::handle(input, gas_limit),
        sign::SELECTOR => sign::handle(input, gas_limit),
        ffi::SELECTOR => ffi::handle(input, gas_limit, state),

        // Unknown VM call: silently drop.
        _ => Some(crate::evm::cheatcode::util::success(gas_limit)),
    }
}
