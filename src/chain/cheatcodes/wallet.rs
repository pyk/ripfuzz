//! Wallet / crypto cheatcodes.

use alloy_primitives::U256;
use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

pub struct Sign;
impl Cheatcode for Sign {
    type Args = (U256, [u8; 32]);
    const SELECTOR: [u8; 4] = [0xe3, 0x41, 0xea, 0xa4];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 + 64 {
            return None;
        }
        let key = U256::from_be_slice(&input[4..36]);
        let mut digest = [0u8; 32];
        digest.copy_from_slice(&input[36..68]);
        Some((key, digest))
    }
    fn effects((key, digest): Self::Args) -> Vec<CheatcodeEffect> {
        let sk_bytes = key.to_be_bytes_vec();
        let signing_key = match k256::ecdsa::SigningKey::from_slice(&sk_bytes) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        let (sig, recid) = match signing_key.sign_prehash_recoverable(&digest) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        let r = sig.r().to_bytes();
        let s = sig.s().to_bytes();
        let v: u8 = if recid.is_y_odd() { 28 } else { 27 };
        let mut output = vec![0u8; 96];
        output[31] = v;
        output[32..64].copy_from_slice(r.as_ref());
        output[64..96].copy_from_slice(s.as_ref());
        vec![CheatcodeEffect::ReturnBytes(output)]
    }
}
