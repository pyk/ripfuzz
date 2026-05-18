//! ECDSA signing cheatcode (`sign`).

use alloy_primitives::U256;
use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

/// secp256k1 curve order (n).
const SECP256K1_ORDER: U256 = U256::from_be_bytes([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
]);

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
        if key.is_zero() {
            return vec![CheatcodeEffect::Revert("private key cannot be 0".into())];
        }
        if key >= SECP256K1_ORDER {
            return vec![CheatcodeEffect::Revert(format!(
                "private key must be less than the secp256k1 curve order ({SECP256K1_ORDER})"
            ))];
        }

        let sk_bytes = key.to_be_bytes_vec();
        let signing_key = match k256::ecdsa::SigningKey::from_slice(&sk_bytes) {
            Ok(v) => v,
            Err(_) => {
                return vec![CheatcodeEffect::Revert("invalid private key".into())];
            }
        };
        let (sig, recid) = match signing_key.sign_prehash_recoverable(&digest) {
            Ok(v) => v,
            Err(_) => {
                return vec![CheatcodeEffect::Revert("signing failed".into())];
            }
        };

        let r = sig.r().to_bytes();
        let s = sig.s().to_bytes();
        let v: u8 = if recid.is_y_odd() { 28 } else { 27 };

        // ABI-encode (uint8 v, bytes32 r, bytes32 s) as three 32-byte words.
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

    use alloy_primitives::keccak256;
    use revm::primitives::Bytes;
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::CheatcodeEffect;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn sign_decode_and_effects() {
        let mut input = vec![0u8; 68];
        input[0..4].copy_from_slice(&Sign::SELECTOR);
        input[35] = 1; // pk = 1
        let digest = keccak256("Data To Sign");
        input[36..68].copy_from_slice(digest.as_ref());

        let args = Sign::decode(&Bytes::from(input)).unwrap();
        let effects = Sign::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        assert_eq!(out.len(), 96);
        let v = out[31];
        assert!(v == 27 || v == 28);
        // r and s are non-zero for a valid signature.
        assert_ne!(&out[32..64], &[0u8; 32]);
        assert_ne!(&out[64..96], &[0u8; 32]);
    }

    #[test]
    fn sign_known_vector_matches_addr() {
        // pk = 1 => addr = 0x7E5F4552091A69125d5DfCb7b8c2659029395Bdf
        let mut input = vec![0u8; 68];
        input[0..4].copy_from_slice(&Sign::SELECTOR);
        input[35] = 1;
        let digest = keccak256("Data To Sign");
        input[36..68].copy_from_slice(digest.as_ref());

        let args = Sign::decode(&Bytes::from(input)).unwrap();
        let effects = Sign::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };

        let v = out[31];
        let r = &out[32..64];
        let s = &out[64..96];

        let signature = alloy_primitives::Signature::from_bytes_and_parity(
            &[r, s].concat(),
            v != 27, // true if v == 28
        );
        let recovered = signature.recover_address_from_prehash(&digest).unwrap();
        assert_eq!(
            recovered.to_string().to_lowercase(),
            "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
        );
    }

    #[test]
    fn sign_zero_reverts() {
        let mut input = vec![0u8; 68];
        input[0..4].copy_from_slice(&Sign::SELECTOR);
        // pk = 0 (leave bytes zero)
        let digest = keccak256("x");
        input[36..68].copy_from_slice(digest.as_ref());

        let args = Sign::decode(&Bytes::from(input)).unwrap();
        let effects = Sign::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::Revert("private key cannot be 0".into())]
        );
    }

    #[test]
    fn sign_too_large_reverts() {
        let mut input = vec![0u8; 68];
        input[0..4].copy_from_slice(&Sign::SELECTOR);
        let order = U256::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C,
            0xD0, 0x36, 0x41, 0x41,
        ]);
        input[4..36].copy_from_slice(&order.to_be_bytes_vec());
        let digest = keccak256("x");
        input[36..68].copy_from_slice(digest.as_ref());

        let args = Sign::decode(&Bytes::from(input)).unwrap();
        let effects = Sign::effects(args);
        assert_eq!(effects.len(), 1);
        let CheatcodeEffect::Revert(msg) = &effects[0] else {
            panic!("expected Revert");
        };
        assert!(msg.contains("private key must be less than the secp256k1 curve order"));
    }

    #[test]
    fn sign_boundary_ok() {
        let mut input = vec![0u8; 68];
        input[0..4].copy_from_slice(&Sign::SELECTOR);
        let order_minus_1 = U256::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C,
            0xD0, 0x36, 0x41, 0x40,
        ]);
        input[4..36].copy_from_slice(&order_minus_1.to_be_bytes_vec());
        let digest = keccak256("x");
        input[36..68].copy_from_slice(digest.as_ref());

        let args = Sign::decode(&Bytes::from(input)).unwrap();
        let effects = Sign::effects(args);
        assert_eq!(effects.len(), 1);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        assert_eq!(out.len(), 96);
        let v = out[31];
        assert!(v == 27 || v == 28);
        assert_ne!(&out[32..64], &[0u8; 32]);
        assert_ne!(&out[64..96], &[0u8; 32]);
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn sign_setup_persists() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let output = chain.execute(&vec![]).unwrap();
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_setup_sign_persists")
            .expect("property should exist");
        assert!(prop.passed, "sign from setUp should persist");
    }

    #[test]
    #[serial]
    fn sign_same_sequence_visibility() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sign_sel: [u8; 4] = [0x2f, 0x64, 0xb9, 0x69]; // call_sign_and_store(uint256)
        let mut args = vec![0u8; 32];
        args[31..32].copy_from_slice(&1u8.to_be_bytes());
        let calls = vec![Call {
            selector: sign_sel,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_sign_visible_in_next_call")
            .expect("property should exist");
        assert!(prop.passed, "signature should be visible in next call");
    }

    #[test]
    #[serial]
    fn sign_revert_undoes_storage() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let revert_sel: [u8; 4] = [0xaa, 0x23, 0xc0, 0x04]; // call_sign_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[31..32].copy_from_slice(&1u8.to_be_bytes());
        let calls = vec![Call {
            selector: revert_sel,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "reverted call should mark all_ok false");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_revert_undoes_storage")
            .expect("property should exist");
        assert!(prop.passed, "storage written before revert must be undone");
    }

    #[test]
    #[serial]
    fn sign_overwrite() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sign_pk_1: [u8; 4] = [0x29, 0xb6, 0x23, 0x1f]; // call_sign_pk_1()
        let sign_pk_2: [u8; 4] = [0x65, 0x0a, 0x2c, 0xc7]; // call_sign_pk_2()
        let calls = vec![
            Call {
                selector: sign_pk_1,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                method_name: String::new(),
                method_signature: String::new(),
                input_values: vec![],
            },
            Call {
                selector: sign_pk_2,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                method_name: String::new(),
                method_signature: String::new(),
                input_values: vec![],
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "calls should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_last_sign_overwrite")
            .expect("property should exist");
        assert!(prop.passed, "last stored signature should be from pk=2");
    }

    #[test]
    #[serial]
    fn sign_zero_reverts_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let zero_sel: [u8; 4] = [0x63, 0x99, 0x58, 0xe0]; // call_sign_zero()
        let calls = vec![Call {
            selector: zero_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "sign(0) should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_sign_zero_reverts")
            .expect("property should exist");
        assert!(prop.passed, "sign(0) revert should leave storage untouched");
    }

    #[test]
    #[serial]
    fn sign_too_large_reverts_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let large_sel: [u8; 4] = [0x03, 0x02, 0x81, 0xa6]; // call_sign_too_large()
        let calls = vec![Call {
            selector: large_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "sign(order) should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_sign_too_large_reverts")
            .expect("property should exist");
        assert!(
            prop.passed,
            "sign(order) revert should leave storage untouched"
        );
    }

    #[test]
    #[serial]
    fn sign_boundary_ok_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let boundary_sel: [u8; 4] = [0x88, 0x75, 0xc4, 0xa5]; // call_sign_boundary()
        let calls = vec![Call {
            selector: boundary_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "sign(order-1) should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_sign_boundary_ok")
            .expect("property should exist");
        assert!(
            prop.passed,
            "sign(order-1) should produce a recoverable signature"
        );
    }

    #[test]
    #[serial]
    fn sign_property_final() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sign_pk_1: [u8; 4] = [0x29, 0xb6, 0x23, 0x1f]; // call_sign_pk_1()
        let calls = vec![Call {
            selector: sign_pk_1,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_final_signature")
            .expect("property should exist");
        assert!(prop.passed, "property should see final stored signature");
    }

    #[test]
    #[serial]
    fn sign_cross_cheatcode() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let cross_sel: [u8; 4] = [0xad, 0x73, 0x11, 0xd5]; // call_sign_and_warp_roll(uint256)
        let mut args = vec![0u8; 32];
        args[31..32].copy_from_slice(&1u8.to_be_bytes());
        let calls = vec![Call {
            selector: cross_sel,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_sign_and_warp_roll")
            .expect("property should exist");
        assert!(
            prop.passed,
            "sign, roll, and warp should coexist without interference"
        );
    }

    #[test]
    #[serial]
    fn sign_different_digest() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let diff_sel: [u8; 4] = [0x4b, 0xb9, 0xc7, 0x75]; // call_sign_different_digest(uint256)
        let mut args = vec![0u8; 32];
        args[31..32].copy_from_slice(&1u8.to_be_bytes());
        let calls = vec![Call {
            selector: diff_sel,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_different_digest_recoverable")
            .expect("property should exist");
        assert!(
            prop.passed,
            "signature for a different digest should still recover correctly"
        );
    }

    #[test]
    #[serial]
    fn sign_corpus_isolation() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeSign.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sign_pk_1: [u8; 4] = [0x29, 0xb6, 0x23, 0x1f]; // call_sign_pk_1()
        let calls_a = vec![Call {
            selector: sign_pk_1,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        let setup_only_sel: [u8; 4] = [0x28, 0x9a, 0x14, 0x34]; // property_setup_sign_persists()
        let calls_b = vec![Call {
            selector: setup_only_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            method_name: String::new(),
            method_signature: String::new(),
            input_values: vec![],
        }];

        let output_b = chain.execute(&calls_b).unwrap();
        assert!(output_b.all_ok, "sequence B should succeed");
        let prop = output_b
            .property_results
            .iter()
            .find(|p| p.name == "property_setup_sign_persists")
            .expect("property should exist");
        assert!(
            prop.passed,
            "stored signature from sequence A must not leak into sequence B"
        );
    }
}
