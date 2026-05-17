//! Wallet / crypto cheatcodes.

use alloy_primitives::{Address, U256};
use revm::interpreter::CallOutcome;

use crate::chain::cheatcodes::{CheatcodeInspector, success_bytes_outcome};

/// `addr(uint256)` returns `address`.
pub const ADDR_SELECTOR: [u8; 4] = [0xf8, 0x63, 0x55, 0x1f];
/// `sign(uint256, bytes32)`.
pub const SIGN_SELECTOR: [u8; 4] = [0x16, 0x00, 0xfc, 0x3e];

pub fn handle_addr(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let value = super::decode_u256_arg(input)?;
    let sk_bytes = value.to_be_bytes_vec();
    let signing_key = match k256::ecdsa::SigningKey::from_slice(&sk_bytes) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let verifying_key = signing_key.verifying_key();
    let public_key = verifying_key.to_encoded_point(false);
    let pk_bytes = public_key.as_bytes();
    if pk_bytes.len() != 65 {
        return None;
    }
    let hash = alloy_primitives::keccak256(&pk_bytes[1..]);
    let address = Address::from_slice(&hash[12..]);

    let mut output = vec![0u8; 32];
    output[12..32].copy_from_slice(address.as_slice());
    Some(success_bytes_outcome(output))
}

pub fn handle_sign(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    if input.len() < 4 + 64 {
        return None;
    }
    let key = U256::from_be_slice(&input[4..36]);
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&input[36..68]);

    let sk_bytes = key.to_be_bytes_vec();
    let signing_key = match k256::ecdsa::SigningKey::from_slice(&sk_bytes) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let (sig, recid) = match signing_key.sign_prehash_recoverable(&digest) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let r = sig.r().to_bytes();
    let s = sig.s().to_bytes();
    let v: u8 = if recid.is_y_odd() { 28 } else { 27 };

    let mut output = vec![0u8; 96];
    output[31] = v;
    output[32..64].copy_from_slice(r.as_ref());
    output[64..96].copy_from_slice(s.as_ref());
    Some(success_bytes_outcome(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::cheatcodes::CheatcodeInspector;

    #[test]
    fn addr_derivation_matches_expected() {
        let mut inspector = CheatcodeInspector::new();
        // Private key = 1
        let mut input = vec![0u8; 36];
        input[0..4].copy_from_slice(&ADDR_SELECTOR);
        input[35] = 1;
        let result = handle_addr(&mut inspector, &revm::primitives::Bytes::from(input));
        assert!(result.is_some());
        let out = result.unwrap().result.output;
        let addr = Address::from_slice(&out[12..32]);
        // Known address for private key 1
        assert_eq!(
            addr.to_string().to_lowercase(),
            "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
        );
    }
}
