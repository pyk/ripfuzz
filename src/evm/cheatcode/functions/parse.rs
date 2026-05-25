//! `parse*` cheatcodes - parse strings into Solidity values.

use revm::primitives::{Address, Bytes, U256};

use crate::evm::cheatcode::util;

pub const PARSE_UINT_SELECTOR: [u8; 4] = [0xfa, 0x91, 0x45, 0x4d];
pub const PARSE_INT_SELECTOR: [u8; 4] = [0x42, 0x34, 0x6c, 0x5e];
pub const PARSE_BOOL_SELECTOR: [u8; 4] = [0x97, 0x4e, 0xf9, 0x24];
pub const PARSE_ADDRESS_SELECTOR: [u8; 4] = [0xc6, 0xce, 0x05, 0x9d];
pub const PARSE_BYTES_SELECTOR: [u8; 4] = [0x8f, 0x5d, 0x23, 0x2d];
pub const PARSE_BYTES32_SELECTOR: [u8; 4] = [0x08, 0x7e, 0x6e, 0x81];

pub fn parse_uint(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let s = util::decode_string(input)?;
    let value: U256 = s.parse().ok()?;
    Some(util::success_u256(value, gas_limit))
}

pub fn parse_int(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let s = util::decode_string(input)?;
    let value: alloy_primitives::I256 = s.parse().ok()?;
    Some(util::success_u256(value.into_raw(), gas_limit))
}

pub fn parse_bool(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let s = util::decode_string(input)?;
    let value = s.trim().eq_ignore_ascii_case("true");
    Some(util::success_bool(value, gas_limit))
}

pub fn parse_address(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let s = util::decode_string(input)?;
    let value: Address = s.parse().ok()?;
    Some(util::success_address(value, gas_limit))
}

pub fn parse_bytes(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let s = util::decode_string(input)?;
    let stripped = s.strip_prefix("0x").unwrap_or(&s);
    let value = hex::decode(stripped).ok()?;
    Some(util::success_bytes(value, gas_limit))
}

pub fn parse_bytes32(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let s = util::decode_string(input)?;
    let stripped = s.strip_prefix("0x").unwrap_or(&s);
    let value = hex::decode(stripped).ok()?;
    if value.len() != 32 {
        return Some(util::revert("parseBytes32: invalid length", gas_limit));
    }
    Some(util::success_bytes(value, gas_limit))
}
