//! `parse*` cheatcodes - parse strings into Solidity values.

use revm::primitives::{Address, U256};

use crate::evm::cheatcode::outcome;

pub fn parse_uint(s: &str, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let value: U256 = s.parse().ok()?;
    Some(outcome::success_u256(value, gas_limit))
}

pub fn parse_int(s: &str, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let value: alloy_primitives::I256 = s.parse().ok()?;
    Some(outcome::success_u256(value.into_raw(), gas_limit))
}

pub fn parse_bool(s: &str, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let value = s.trim().eq_ignore_ascii_case("true");
    Some(outcome::success_bool(value, gas_limit))
}

pub fn parse_address(s: &str, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let value: Address = s.parse().ok()?;
    Some(outcome::success_address(value, gas_limit))
}

pub fn parse_bytes(s: &str, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    let value = hex::decode(stripped).ok()?;
    Some(outcome::success_bytes(value, gas_limit))
}

pub fn parse_bytes32(s: &str, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    let value = hex::decode(stripped).ok()?;
    if value.len() != 32 {
        return Some(outcome::revert("parseBytes32: invalid length", gas_limit));
    }
    Some(outcome::success_bytes(value, gas_limit))
}
