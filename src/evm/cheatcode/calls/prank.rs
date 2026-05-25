//! Prank cheatcodes - `vm.prank`, `vm.startPrank`, `vm.stopPrank`.

use revm::primitives::Address;

use crate::evm::cheatcode::{
    outcome,
    state::{ExecutionState, PrankState, StartPrankState},
};

pub fn prank(state: &mut ExecutionState, addr: Address) -> Option<revm::interpreter::CallOutcome> {
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(outcome::revert(
            "prank(address) cannot be called when a prank is already active",
        ));
    }
    if state.prank.start.is_some() {
        return Some(outcome::revert(
            "prank(address) cannot be called when a startPrank is already active",
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
    Some(outcome::success())
}

pub fn prank_origin(
    state: &mut ExecutionState,
    addr: Address,
    origin: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let origin = Some(origin);
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(outcome::revert(
            "prank(address) cannot be called when a prank is already active",
        ));
    }
    if state.prank.start.is_some() {
        return Some(outcome::revert(
            "prank(address) cannot be called when a startPrank is already active",
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
    Some(outcome::success())
}

pub fn start_prank(
    state: &mut ExecutionState,
    addr: Address,
) -> Option<revm::interpreter::CallOutcome> {
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(outcome::revert(
            "startPrank(address) cannot be called when a prank is already active",
        ));
    }
    if let Some(ref start) = state.prank.start
        && !start.used
    {
        return Some(outcome::revert(
            "startPrank(address) cannot be called when a startPrank is already active",
        ));
    }
    state.prank.start = Some(StartPrankState {
        caller: addr,
        origin: None,
        prank_caller: Address::ZERO,
        set_depth: 0,
        used: false,
    });
    Some(outcome::success())
}

pub fn start_prank_origin(
    state: &mut ExecutionState,
    addr: Address,
    origin: Address,
) -> Option<revm::interpreter::CallOutcome> {
    let origin = Some(origin);
    if let Some(ref active) = state.prank.active
        && !active.used
    {
        return Some(outcome::revert(
            "startPrank(address) cannot be called when a prank is already active",
        ));
    }
    if let Some(ref start) = state.prank.start
        && !start.used
    {
        return Some(outcome::revert(
            "startPrank(address) cannot be called when a startPrank is already active",
        ));
    }
    state.prank.start = Some(StartPrankState {
        caller: addr,
        origin,
        prank_caller: Address::ZERO,
        set_depth: 0,
        used: false,
    });
    Some(outcome::success())
}

pub fn stop_prank(state: &mut ExecutionState) -> Option<revm::interpreter::CallOutcome> {
    state.prank.active = None;
    state.prank.start = None;
    Some(outcome::success())
}
