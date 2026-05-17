//! Wallet / crypto cheatcodes.

use alloy_primitives::{Address, U256};
use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect, decode_u256_arg};

pub struct Addr;
impl Cheatcode for Addr {
    type Args = U256;
    const SELECTOR: [u8; 4] = [0xff, 0xa1, 0x86, 0x49];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_u256_arg(input)
    }
    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        let sk_bytes = value.to_be_bytes_vec();
        let signing_key = match k256::ecdsa::SigningKey::from_slice(&sk_bytes) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        let verifying_key = signing_key.verifying_key();
        let public_key = verifying_key.to_encoded_point(false);
        let pk_bytes = public_key.as_bytes();
        if pk_bytes.len() != 65 {
            return vec![];
        }
        let hash = alloy_primitives::keccak256(&pk_bytes[1..]);
        let address = Address::from_slice(&hash[12..]);
        let mut output = vec![0u8; 32];
        output[12..32].copy_from_slice(address.as_slice());
        vec![CheatcodeEffect::ReturnBytes(output)]
    }
}

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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;

    #[test]
    fn addr_derivation_matches_expected() {
        let mut input = vec![0u8; 36];
        input[0..4].copy_from_slice(&Addr::SELECTOR);
        input[35] = 1;
        let args = Addr::decode(&Bytes::from(input)).unwrap();
        let effects = Addr::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let addr = Address::from_slice(&out[12..32]);
        assert_eq!(
            addr.to_string().to_lowercase(),
            "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
        );
    }

    #[test]
    fn cheatcode_wallet_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWallet.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let output = chain.execute(&vec![]).unwrap();
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "wallet property should pass"
        );
    }
}
