//! `addr` cheatcode - derive an address from a private key.

use alloy_primitives::{Address, U256};
use revm::primitives::Bytes;

use crate::evm::cheatcode::util;

pub const SELECTOR: [u8; 4] = [0xff, 0xa1, 0x86, 0x49];

/// secp256k1 curve order (n).
const SECP256K1_ORDER: U256 = U256::from_be_bytes([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
]);

pub fn handle(input: &Bytes, gas_limit: u64) -> Option<revm::interpreter::CallOutcome> {
    let sk = util::decode_u256(input)?;
    if sk.is_zero() {
        return Some(util::revert("private key cannot be 0", gas_limit));
    }
    if sk >= SECP256K1_ORDER {
        return Some(util::revert(
            &format!("private key must be less than the secp256k1 curve order ({SECP256K1_ORDER})"),
            gas_limit,
        ));
    }
    let sk_bytes = sk.to_be_bytes_vec();
    let signing_key = k256::ecdsa::SigningKey::from_slice(&sk_bytes).ok()?;
    let verifying_key = signing_key.verifying_key();
    let public_key = verifying_key.to_encoded_point(false);
    let pk_bytes = public_key.as_bytes();
    if pk_bytes.len() != 65 {
        return Some(util::revert("invalid public key length", gas_limit));
    }
    let hash = alloy_primitives::keccak256(&pk_bytes[1..]);
    let address = Address::from_slice(&hash[12..]);
    Some(util::success_address(address, gas_limit))
}
