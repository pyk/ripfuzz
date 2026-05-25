//! `sign` cheatcode - sign a digest with a private key.

use alloy_primitives::U256;

use crate::evm::cheatcode::outcome;

/// secp256k1 curve order (n).
const SECP256K1_ORDER: U256 = U256::from_be_bytes([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
]);

pub fn handle(sk: U256, digest: [u8; 32]) -> Option<revm::interpreter::CallOutcome> {
    if sk.is_zero() {
        return Some(outcome::revert("private key cannot be 0"));
    }
    if sk >= SECP256K1_ORDER {
        return Some(outcome::revert(&format!(
            "private key must be less than the secp256k1 curve order ({SECP256K1_ORDER})"
        )));
    }
    let sk_bytes = sk.to_be_bytes_vec();
    let signing_key = k256::ecdsa::SigningKey::from_slice(&sk_bytes).ok()?;
    let (sig, recid) = signing_key.sign_prehash_recoverable(&digest).ok()?;
    let r = sig.r().to_bytes();
    let s = sig.s().to_bytes();
    let v: u8 = if recid.is_y_odd() { 28 } else { 27 };
    let r_arr: [u8; 32] = AsRef::<[u8]>::as_ref(&r).try_into().ok()?;
    let s_arr: [u8; 32] = AsRef::<[u8]>::as_ref(&s).try_into().ok()?;
    Some(outcome::success_sign(v, r_arr, s_arr))
}
