//! Prank cheatcodes - `vm.prank`, `vm.startPrank`, `vm.stopPrank`.

use revm::primitives::Address;

use crate::evm::cheatcode::{
    state::{ExecutionState, PrankState, StartPrankState},
    util,
};

pub fn prank(
    addr: Address,
    gas_limit: u64,
    _ctx: &mut impl revm::context_interface::ContextTr,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
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
    addr: Address,
    origin: Address,
    gas_limit: u64,
    _ctx: &mut impl revm::context_interface::ContextTr,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let origin = Some(origin);
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
    addr: Address,
    gas_limit: u64,
    _ctx: &mut impl revm::context_interface::ContextTr,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
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
    addr: Address,
    origin: Address,
    gas_limit: u64,
    _ctx: &mut impl revm::context_interface::ContextTr,
    state: &mut ExecutionState,
) -> Option<revm::interpreter::CallOutcome> {
    let origin = Some(origin);
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
