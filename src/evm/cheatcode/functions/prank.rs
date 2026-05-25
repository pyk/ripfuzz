//! Prank cheatcodes - `vm.prank`, `vm.startPrank`, `vm.stopPrank`.

use revm::primitives::{Address, Bytes};

use crate::evm::cheatcode::{
    state::{ExecutionState, PrankState, StartPrankState},
    util,
};

pub const PRANK_SELECTOR: [u8; 4] = [0xca, 0x66, 0x9f, 0xa7];
pub const PRANK_ORIGIN_SELECTOR: [u8; 4] = [0x47, 0xe5, 0x0c, 0xce];
pub const START_PRANK_SELECTOR: [u8; 4] = [0x06, 0x44, 0x7d, 0x56];
pub const START_PRANK_ORIGIN_SELECTOR: [u8; 4] = [0x45, 0xb5, 0x60, 0x78];
pub const STOP_PRANK: [u8; 4] = [0x90, 0xc5, 0x01, 0x3b];

pub fn prank(
    input: &Bytes,
    gas_limit: u64,
    _ctx: &mut impl revm::context_interface::ContextTr,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let addr = util::decode_address(input)?;
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(util::revert(
            "prank(address) cannot be called when a prank is already active",
            gas_limit,
        ));
    }
    if state.prank.start.is_some() {
        return Some(util::revert(
            "prank(address) cannot be called when a startPrank is already active",
            gas_limit,
        ));
    }
    state.prank.active = Some(PrankState {
        caller: addr,
        origin: None,
        prank_caller: Address::ZERO,
        set_depth: 0,
        single_call: true,
        used: false,
    });
    Some(util::success(gas_limit))
}

pub fn prank_origin(
    input: &Bytes,
    gas_limit: u64,
    _ctx: &mut impl revm::context_interface::ContextTr,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let (addr, origin) = util::decode_address_u256(input)?;
    let origin = Some(revm::primitives::Address::from_word(origin.into()));
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(util::revert(
            "prank(address) cannot be called when a prank is already active",
            gas_limit,
        ));
    }
    if state.prank.start.is_some() {
        return Some(util::revert(
            "prank(address) cannot be called when a startPrank is already active",
            gas_limit,
        ));
    }
    state.prank.active = Some(PrankState {
        caller: addr,
        origin,
        prank_caller: Address::ZERO,
        set_depth: 0,
        single_call: true,
        used: false,
    });
    Some(util::success(gas_limit))
}

pub fn start_prank(
    input: &Bytes,
    gas_limit: u64,
    _ctx: &mut impl revm::context_interface::ContextTr,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let addr = util::decode_address(input)?;
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(util::revert(
            "startPrank(address) cannot be called when a prank is already active",
            gas_limit,
        ));
    }
    if let Some(ref start) = state.prank.start
        && !start.used
    {
        return Some(util::revert(
            "startPrank(address) cannot be called when a startPrank is already active",
            gas_limit,
        ));
    }
    state.prank.start = Some(StartPrankState {
        caller: addr,
        origin: None,
        prank_caller: Address::ZERO,
        set_depth: 0,
        used: false,
    });
    Some(util::success(gas_limit))
}

pub fn start_prank_origin(
    input: &Bytes,
    gas_limit: u64,
    _ctx: &mut impl revm::context_interface::ContextTr,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let (addr, origin) = util::decode_address_u256(input)?;
    let origin = Some(revm::primitives::Address::from_word(origin.into()));
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(util::revert(
            "startPrank(address) cannot be called when a prank is already active",
            gas_limit,
        ));
    }
    if let Some(ref start) = state.prank.start
        && !start.used
    {
        return Some(util::revert(
            "startPrank(address) cannot be called when a startPrank is already active",
            gas_limit,
        ));
    }
    state.prank.start = Some(StartPrankState {
        caller: addr,
        origin,
        prank_caller: Address::ZERO,
        set_depth: 0,
        used: false,
    });
    Some(util::success(gas_limit))
}

pub fn stop_prank(
    gas_limit: u64,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    state.prank.active = None;
    state.prank.start = None;
    Some(util::success(gas_limit))
}
