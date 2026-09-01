//! Cheatcode handlers - one submodule per Foundry-compatible cheatcode.

use alloy_sol_types::sol;
use revm::{
    context::BlockEnv, context::ContextSetters, context_interface::ContextTr,
    interpreter::CallOutcome,
};

use Vm::VmCalls;

use crate::evm::cheatcode::inspector::CfgMut;
use crate::evm::cheatcode::state::ExecutionState;
use crate::evm::database::DatabaseExt;

pub mod addr;
pub mod bail;
pub mod chain_id;
pub mod coinbase;
pub mod deal;
pub mod etch;
pub mod fee;
pub mod ffi;
pub mod fork;
pub mod get_env;
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
    interface Vm {
        // Block
        function warp(uint256 newTimestamp) external;
        function roll(uint256 newNumber) external;
        function fee(uint256 newBasefee) external;
        function coinbase(address newCoinbase) external;
        function prevrandao(bytes32 newPrevrandao) external;
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

        // Wallet / ffi
        function addr(uint256 sk) external returns (address);
        function sign(uint256 sk, bytes32 digest) external returns (uint8 v, bytes32 r, bytes32 s);
        function ffi(string[] args) external returns (bytes memory);

        // Environment
        function getEnv(string key) external returns (string memory value);
        function getEnv(string key, string defaultValue) external returns (string memory value);

        // Fork
        struct ForkConfig {
            uint32 retries;
            uint64 backoffMs;
            uint64 timeoutMs;
            uint64 rateLimit;
        }
        function fork(string url, uint256 blockNumber) external;
        function fork(string url, uint256 blockNumber, ForkConfig config) external;

        // Invariant
        struct Invariant { string id; string description; }
        function bail(Invariant calldata invariant) external;
    }
}

/// Dispatch a decoded cheatcode call to its handler.
pub fn dispatch<CTX>(
    call: VmCalls,

    ctx: &mut CTX,
    state: &mut ExecutionState,
) -> Option<CallOutcome>
where
    CTX: ContextTr + ContextSetters<Block = BlockEnv> + CfgMut,
    CTX::Db: DatabaseExt + fork::AsForkDatabase,
    CTX::Journal: fork::CommitRemoteBeforeForkSwitch,
{
    match call {
        // Block
        VmCalls::warp(c) => warp::handle(ctx, state, c.newTimestamp),
        VmCalls::roll(c) => roll::handle(ctx, state, c.newNumber),
        VmCalls::fee(c) => fee::handle(ctx, state, c.newBasefee),
        VmCalls::coinbase(c) => coinbase::handle(ctx, state, c.newCoinbase),
        VmCalls::prevrandao(c) => prevrandao::handle(ctx, state, c.newPrevrandao.into()),
        VmCalls::chainId(c) => chain_id::handle(ctx, state, c.newChainId),

        // Account
        VmCalls::deal(c) => deal::handle(ctx, c.account, c.value),
        VmCalls::etch(c) => etch::handle(ctx, c.account, c.code),
        VmCalls::setNonce(c) => nonce::set_nonce(ctx, c.account, c.nonce),
        VmCalls::getNonce(c) => nonce::get_nonce(ctx, c.account),
        VmCalls::store(c) => storage::store(ctx, c.account, c.slot.into(), c.value.into()),
        VmCalls::load(c) => storage::load(ctx, c.account, c.slot.into()),

        // Prank
        VmCalls::prank_0(c) => prank::prank(state, c.a),
        VmCalls::prank_1(c) => prank::prank_origin(state, c.a, c.origin),
        VmCalls::startPrank_0(c) => prank::start_prank(state, c.a),
        VmCalls::startPrank_1(c) => prank::start_prank_origin(state, c.a, c.origin),
        VmCalls::stopPrank(_) => prank::stop_prank(state),

        // Label
        VmCalls::label(c) => label::label(state, c.account, &c.name),
        VmCalls::getLabel(c) => label::get_label(state, c.account),

        // Conversion
        VmCalls::toString_0(c) => to_string::to_string_address(c.a),
        VmCalls::toString_1(c) => to_string::to_string_bool(c.b),
        VmCalls::toString_2(c) => to_string::to_string_uint(c.v),
        VmCalls::toString_3(c) => to_string::to_string_int(c.v),
        VmCalls::toString_4(c) => to_string::to_string_bytes32(c.b.into()),
        VmCalls::toString_5(c) => to_string::to_string_bytes(c.b),
        VmCalls::parseUint(c) => parse::parse_uint(&c.s),
        VmCalls::parseInt(c) => parse::parse_int(&c.s),
        VmCalls::parseBool(c) => parse::parse_bool(&c.s),
        VmCalls::parseAddress(c) => parse::parse_address(&c.s),
        VmCalls::parseBytes(c) => parse::parse_bytes(&c.s),
        VmCalls::parseBytes32(c) => parse::parse_bytes32(&c.s),

        // Wallet / ffi
        VmCalls::addr(c) => addr::handle(c.sk),
        VmCalls::sign(c) => sign::handle(c.sk, c.digest.into()),
        VmCalls::ffi(c) => ffi::handle(c.args, state),

        // Environment
        VmCalls::getEnv_0(c) => get_env::get_env(&c.key),
        VmCalls::getEnv_1(c) => get_env::get_env_or_default(&c.key, &c.defaultValue),

        // Fork
        VmCalls::fork_0(c) => fork::fork(ctx, state, &c.url, c.blockNumber),
        VmCalls::fork_1(c) => fork::fork_with_options(
            ctx,
            state,
            &c.url,
            c.blockNumber,
            fork::ForkOptions {
                retries: Some(c.config.retries),
                backoff_ms: Some(c.config.backoffMs),
                timeout_ms: Some(c.config.timeoutMs),
                rate_limit: Some(c.config.rateLimit),
            },
        ),

        // Invariant
        VmCalls::bail(c) => bail::handle(state, c.invariant),
    }
}
