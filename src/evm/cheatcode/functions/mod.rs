//! Cheatcode handlers - one submodule per Foundry-compatible cheatcode.

use alloy_sol_types::sol;
use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr,
    interpreter::CallOutcome,
};

use Cheatcodes::CheatcodesCalls;

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

sol! {
    interface Cheatcodes {
        // Block
        function warp(uint256 newTimestamp) external;
        function roll(uint256 newNumber) external;
        function fee(uint256 newBasefee) external;
        function coinbase(address newCoinbase) external;
        function prevrandao(bytes32 newPrevrandao) external;
        function difficulty(uint256 newDifficulty) external;
        function chainId(uint256 newChainId) external;

        // Account
        function deal(address account, uint256 value) external;
        function etch(address account, bytes code) external;
        function setNonce(address account, uint64 nonce) external;
        function getNonce(address account) external returns (uint256);
        function store(address account, bytes32 slot, bytes32 value) external;
        function load(address account, bytes32 slot) external returns (bytes32);

        // Prank
        function prank(address a) external;
        function prank(address a, address origin) external;
        function startPrank(address a) external;
        function startPrank(address a, address origin) external;
        function stopPrank() external;

        // Label
        function label(address account, string name) external;
        function getLabel(address account) external returns (string memory);

        // Conversion
        function toString(address a) external returns (string memory);
        function toString(bool b) external returns (string memory);
        function toString(uint256 v) external returns (string memory);
        function toString(int256 v) external returns (string memory);
        function toString(bytes32 b) external returns (string memory);
        function toString(bytes b) external returns (string memory);
        function parseUint(string s) external returns (uint256);
        function parseInt(string s) external returns (int256);
        function parseBool(string s) external returns (bool);
        function parseAddress(string s) external returns (address);
        function parseBytes(string s) external returns (bytes memory);
        function parseBytes32(string s) external returns (bytes32);

        // Code / wallet / ffi
        function getCode(string name) external returns (bytes memory);
        function addr(uint256 sk) external returns (address);
        function sign(uint256 sk, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
        function ffi(string[] args) external returns (bytes memory);
    }
}

/// Dispatch a decoded cheatcode call to its handler.
pub fn dispatch<CTX>(
    call: CheatcodesCalls,
    gas_limit: u64,
    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<CallOutcome>
where
    CTX: ContextTr + ContextSetters<Block = BlockEnv> + crate::evm::cheatcode::inspector::CfgMut,
{
    match call {
        // Block
        CheatcodesCalls::warp(c) => warp::handle(c.newTimestamp, gas_limit, ctx, state),
        CheatcodesCalls::roll(c) => roll::handle(c.newNumber, gas_limit, ctx, state),
        CheatcodesCalls::fee(c) => fee::handle(c.newBasefee, gas_limit, ctx, state),
        CheatcodesCalls::coinbase(c) => coinbase::handle(c.newCoinbase, gas_limit, ctx, state),
        CheatcodesCalls::prevrandao(c) => {
            prevrandao::handle(c.newPrevrandao.into(), gas_limit, ctx, state)
        }
        CheatcodesCalls::difficulty(c) => {
            difficulty::handle(c.newDifficulty, gas_limit, ctx, state)
        }
        CheatcodesCalls::chainId(c) => chain_id::handle(c.newChainId, gas_limit, ctx, state),

        // Account
        CheatcodesCalls::deal(c) => deal::handle(c.account, c.value, gas_limit, ctx, state),
        CheatcodesCalls::etch(c) => etch::handle(c.account, c.code, gas_limit, ctx, state),
        CheatcodesCalls::setNonce(c) => nonce::set_nonce(c.account, c.nonce, gas_limit, ctx, state),
        CheatcodesCalls::getNonce(c) => nonce::get_nonce(c.account, gas_limit, ctx, state),
        CheatcodesCalls::store(c) => storage::store(
            c.account,
            c.slot.into(),
            c.value.into(),
            gas_limit,
            ctx,
            state,
        ),
        CheatcodesCalls::load(c) => storage::load(c.account, c.slot.into(), gas_limit, ctx, state),

        // Prank
        CheatcodesCalls::prank_0(c) => prank::prank(c.a, gas_limit, ctx, state),
        CheatcodesCalls::prank_1(c) => prank::prank_origin(c.a, c.origin, gas_limit, ctx, state),
        CheatcodesCalls::startPrank_0(c) => prank::start_prank(c.a, gas_limit, ctx, state),
        CheatcodesCalls::startPrank_1(c) => {
            prank::start_prank_origin(c.a, c.origin, gas_limit, ctx, state)
        }
        CheatcodesCalls::stopPrank(_) => prank::stop_prank(gas_limit, state),

        // Label
        CheatcodesCalls::label(c) => label::label(c.account, &c.name, gas_limit, state),
        CheatcodesCalls::getLabel(c) => label::get_label(c.account, gas_limit, state),

        // Conversion
        CheatcodesCalls::toString_0(c) => to_string::to_string_address(c.a, gas_limit),
        CheatcodesCalls::toString_1(c) => to_string::to_string_bool(c.b, gas_limit),
        CheatcodesCalls::toString_2(c) => to_string::to_string_uint(c.v, gas_limit),
        CheatcodesCalls::toString_3(c) => to_string::to_string_int(c.v, gas_limit),
        CheatcodesCalls::toString_4(c) => to_string::to_string_bytes32(c.b.into(), gas_limit),
        CheatcodesCalls::toString_5(c) => to_string::to_string_bytes(c.b, gas_limit),
        CheatcodesCalls::parseUint(c) => parse::parse_uint(&c.s, gas_limit),
        CheatcodesCalls::parseInt(c) => parse::parse_int(&c.s, gas_limit),
        CheatcodesCalls::parseBool(c) => parse::parse_bool(&c.s, gas_limit),
        CheatcodesCalls::parseAddress(c) => parse::parse_address(&c.s, gas_limit),
        CheatcodesCalls::parseBytes(c) => parse::parse_bytes(&c.s, gas_limit),
        CheatcodesCalls::parseBytes32(c) => parse::parse_bytes32(&c.s, gas_limit),

        // Code / wallet / ffi
        CheatcodesCalls::getCode(c) => get_code::handle(&c.name, gas_limit, state),
        CheatcodesCalls::addr(c) => addr::handle(c.sk, gas_limit),
        CheatcodesCalls::sign(c) => sign::handle(c.sk, c.digest.into(), gas_limit),
        CheatcodesCalls::ffi(c) => ffi::handle(c.args, gas_limit, state),
    }
}
