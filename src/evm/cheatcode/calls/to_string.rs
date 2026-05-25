//! `toString` cheatcodes - convert Solidity values to strings.

use alloy_dyn_abi::DynSolValue;
use alloy_primitives::I256;
use revm::primitives::{Address, Bytes, U256};

use crate::evm::cheatcode::outcome;

fn to_string_outcome(s: &str) -> Option<revm::interpreter::CallOutcome> {
    let encoded = DynSolValue::String(s.to_owned()).abi_encode();
    Some(outcome::success_bytes(encoded))
}

pub fn to_string_address(addr: Address) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("{:?}", addr);
    to_string_outcome(&s)
}

pub fn to_string_bool(b: bool) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("{b}");
    to_string_outcome(&s)
}

pub fn to_string_uint(value: U256) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("{value}");
    to_string_outcome(&s)
}

pub fn to_string_int(value: I256) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("{value}");
    to_string_outcome(&s)
}

pub fn to_string_bytes32(b: [u8; 32]) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("0x{}", hex::encode(b));
    to_string_outcome(&s)
}

pub fn to_string_bytes(b: Bytes) -> Option<revm::interpreter::CallOutcome> {
    let s = format!("0x{}", hex::encode(b));
    to_string_outcome(&s)
}
