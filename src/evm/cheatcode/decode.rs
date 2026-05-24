//! Calldata decoders for cheatcode arguments.

use revm::primitives::{Address, Bytes, U256};

pub fn decode_u256_arg(input: &Bytes) -> Option<U256> {
    if input.len() < 4 + 32 {
        return None;
    }
    Some(U256::from_be_slice(&input[4..36]))
}

pub fn decode_address_arg(input: &Bytes) -> Option<Address> {
    if input.len() < 4 + 32 {
        return None;
    }
    Some(Address::from_slice(&input[4 + 12..4 + 32]))
}

pub fn decode_address_u256_args(input: &Bytes) -> Option<(Address, U256)> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let value = U256::from_be_slice(&input[4 + 32..4 + 64]);
    Some((addr, value))
}

pub fn decode_address_bytes32_bytes32_args(input: &Bytes) -> Option<(Address, [u8; 32], [u8; 32])> {
    if input.len() < 4 + 96 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let mut slot = [0u8; 32];
    slot.copy_from_slice(&input[4 + 32..4 + 64]);
    let mut value = [0u8; 32];
    value.copy_from_slice(&input[4 + 64..4 + 96]);
    Some((addr, slot, value))
}

pub fn decode_address_bytes32_args(input: &Bytes) -> Option<(Address, [u8; 32])> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let mut slot = [0u8; 32];
    slot.copy_from_slice(&input[4 + 32..4 + 64]);
    Some((addr, slot))
}
