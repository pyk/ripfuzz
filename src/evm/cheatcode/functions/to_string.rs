//! `toString` cheatcodes - convert Solidity values to strings.

use alloy_dyn_abi::DynSolValue;
use revm::primitives::Bytes;

use crate::evm::cheatcode::util;

pub const TO_STRING_ADDRESS_SELECTOR: [u8; 4] = [0x56, 0xca, 0x62, 0x3e];
pub const TO_STRING_BOOL_SELECTOR: [u8; 4] = [0x71, 0xdc, 0xe7, 0xda];
pub const TO_STRING_UINT_SELECTOR: [u8; 4] = [0x69, 0x00, 0xa3, 0xae];
pub const TO_STRING_INT_SELECTOR: [u8; 4] = [0xa3, 0x22, 0xc4, 0x0e];
pub const TO_STRING_BYTES32_SELECTOR: [u8; 4] = [0xb1, 0x1a, 0x19, 0xe8];
pub const TO_STRING_BYTES_SELECTOR: [u8; 4] = [0x71, 0xaa, 0xd1, 0x0d];

fn to_string_outcome(s: &str, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let encoded = DynSolValue::String(s.to_owned()).abi_encode();
    Some(util::success_bytes(encoded, gas_limit))
}

pub fn to_string_address(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let addr = util::decode_address(input)?;
    let s = format!("{:?}", addr);
    to_string_outcome(&s, gas_limit)
}

pub fn to_string_bool(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let mut vals = util::decode_params(input, &[alloy_dyn_abi::DynSolType::Bool])?;
    let alloy_dyn_abi::DynSolValue::Bool(b) = vals.pop()? else {
        return None;
    };
    let s = format!("{b}");
    to_string_outcome(&s, gas_limit)
}

pub fn to_string_uint(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let value = util::decode_u256(input)?;
    let s = format!("{value}");
    to_string_outcome(&s, gas_limit)
}

pub fn to_string_int(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let value = util::decode_u256(input)?;
    let signed = alloy_primitives::I256::from_raw(value);
    let s = format!("{signed}");
    to_string_outcome(&s, gas_limit)
}

pub fn to_string_bytes32(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let (_, b) = util::decode_address_bytes32(input)?;
    let s = format!("0x{}", hex::encode(b));
    to_string_outcome(&s, gas_limit)
}

pub fn to_string_bytes(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let mut vals = util::decode_params(input, &[alloy_dyn_abi::DynSolType::Bytes])?;
    let alloy_dyn_abi::DynSolValue::Bytes(b) = vals.pop()? else {
        return None;
    };
    let s = format!("0x{}", hex::encode(b));
    to_string_outcome(&s, gas_limit)
}
