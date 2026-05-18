//! Address derivation cheatcode (`addr`).

use alloy_primitives::{Address, U256};
use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect, decode_u256_arg};

/// secp256k1 curve order (n).
const SECP256K1_ORDER: U256 = U256::from_be_bytes([
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
]);

pub struct Addr;
impl Cheatcode for Addr {
    type Args = U256;
    const SELECTOR: [u8; 4] = [0xff, 0xa1, 0x86, 0x49];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_u256_arg(input)
    }

    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        if value.is_zero() {
            return vec![CheatcodeEffect::Revert("private key cannot be 0".into())];
        }
        if value >= SECP256K1_ORDER {
            return vec![CheatcodeEffect::Revert(format!(
                "private key must be less than the secp256k1 curve order ({SECP256K1_ORDER})"
            ))];
        }

        let sk_bytes = value.to_be_bytes_vec();
        let signing_key = match k256::ecdsa::SigningKey::from_slice(&sk_bytes) {
            Ok(v) => v,
            Err(_) => {
                // This branch is defensive; the range checks above should catch
                // every invalid key that k256 would reject.
                return vec![CheatcodeEffect::Revert("invalid private key".into())];
            }
        };
        let verifying_key = signing_key.verifying_key();
        let public_key = verifying_key.to_encoded_point(false);
        let pk_bytes = public_key.as_bytes();
        if pk_bytes.len() != 65 {
            return vec![CheatcodeEffect::Revert("invalid public key length".into())];
        }
        let hash = alloy_primitives::keccak256(&pk_bytes[1..]);
        let address = Address::from_slice(&hash[12..]);
        let mut output = vec![0u8; 32];
        output[12..32].copy_from_slice(address.as_slice());
        vec![CheatcodeEffect::ReturnBytes(output)]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::primitives::Bytes;
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::CheatcodeEffect;
    use crate::contract;
    use crate::corpus::Call;

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
    fn addr_zero_reverts() {
        let mut input = vec![0u8; 36];
        input[0..4].copy_from_slice(&Addr::SELECTOR);
        let args = Addr::decode(&Bytes::from(input)).unwrap();
        let effects = Addr::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::Revert("private key cannot be 0".into())]
        );
    }

    #[test]
    fn addr_too_large_reverts() {
        let mut input = vec![0u8; 36];
        input[0..4].copy_from_slice(&Addr::SELECTOR);
        // secp256k1_order
        let order = U256::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C,
            0xD0, 0x36, 0x41, 0x41,
        ]);
        input[4..36].copy_from_slice(&order.to_be_bytes_vec());
        let args = Addr::decode(&Bytes::from(input)).unwrap();
        let effects = Addr::effects(args);
        assert_eq!(effects.len(), 1);
        let CheatcodeEffect::Revert(msg) = &effects[0] else {
            panic!("expected Revert");
        };
        assert!(msg.contains("private key must be less than the secp256k1 curve order"));
    }

    #[test]
    fn addr_boundary_ok() {
        let mut input = vec![0u8; 36];
        input[0..4].copy_from_slice(&Addr::SELECTOR);
        let order_minus_1 = U256::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C,
            0xD0, 0x36, 0x41, 0x40,
        ]);
        input[4..36].copy_from_slice(&order_minus_1.to_be_bytes_vec());
        let args = Addr::decode(&Bytes::from(input)).unwrap();
        let effects = Addr::effects(args);
        assert_eq!(effects.len(), 1);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let addr = Address::from_slice(&out[12..32]);
        // Just ensure it is not zero and has the correct length.
        assert_ne!(addr, Address::ZERO);
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn addr_setup_persists() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let output = chain.execute(&vec![]).unwrap();
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_setup_addr_persists")
            .expect("property should exist");
        assert!(prop.passed, "addr from setUp should persist");
    }

    #[test]
    #[serial]
    fn addr_same_sequence_visibility() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let derive_sel: [u8; 4] = [0xd3, 0x69, 0x6f, 0x28]; // call_derive_and_store(uint256)
        let mut args = vec![0u8; 32];
        args[30..32].copy_from_slice(&100u16.to_be_bytes());
        let calls = vec![Call {
            selector: derive_sel,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_addr_visible_in_next_call")
            .expect("property should exist");
        assert!(prop.passed, "addr result should be visible in next call");
    }

    #[test]
    #[serial]
    fn addr_revert_undoes_storage() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let revert_sel: [u8; 4] = [0x93, 0x5c, 0x8b, 0x89]; // call_derive_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[31..32].copy_from_slice(&1u8.to_be_bytes());
        let calls = vec![Call {
            selector: revert_sel,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        // The top-level call reverts, so all_ok should be false.
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
    fn addr_overwrite() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let store_pk_1: [u8; 4] = [0x82, 0xbd, 0xfc, 0xe9]; // call_store_pk_1()
        let store_pk_2: [u8; 4] = [0xbb, 0xe0, 0xc9, 0xc7]; // call_store_pk_2()
        let calls = vec![
            Call {
                selector: store_pk_1,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: store_pk_2,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "calls should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_last_addr_overwrite")
            .expect("property should exist");
        assert!(prop.passed, "last stored addr should be from pk=2");
    }

    #[test]
    #[serial]
    fn addr_zero_reverts_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let zero_sel: [u8; 4] = [0x02, 0x2c, 0xe4, 0xce]; // call_addr_zero()
        let calls = vec![Call {
            selector: zero_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "addr(0) should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_addr_zero_reverts")
            .expect("property should exist");
        assert!(prop.passed, "addr(0) revert should leave storage untouched");
    }

    #[test]
    #[serial]
    fn addr_too_large_reverts_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let large_sel: [u8; 4] = [0x08, 0xe8, 0x98, 0xab]; // call_addr_too_large()
        let calls = vec![Call {
            selector: large_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "addr(order) should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_addr_too_large_reverts")
            .expect("property should exist");
        assert!(
            prop.passed,
            "addr(order) revert should leave storage untouched"
        );
    }

    #[test]
    #[serial]
    fn addr_boundary_ok_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let boundary_sel: [u8; 4] = [0x50, 0x0f, 0x60, 0x92]; // call_addr_boundary()
        let calls = vec![Call {
            selector: boundary_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "addr(order-1) should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_addr_boundary_ok")
            .expect("property should exist");
        assert!(prop.passed, "addr(order-1) should produce a valid address");
    }

    #[test]
    #[serial]
    fn addr_property_final() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let store_sel: [u8; 4] = [0x82, 0xbd, 0xfc, 0xe9]; // call_store_pk_1()
        let calls = vec![Call {
            selector: store_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_final_addr")
            .expect("property should exist");
        assert!(prop.passed, "property should see final stored address");
    }

    #[test]
    #[serial]
    fn addr_cross_cheatcode() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let cross_sel: [u8; 4] = [0x59, 0x76, 0x0b, 0x50]; // call_addr_and_warp_roll(uint256)
        let mut args = vec![0u8; 32];
        args[31..32].copy_from_slice(&1u8.to_be_bytes());
        let calls = vec![Call {
            selector: cross_sel,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_addr_and_warp_roll")
            .expect("property should exist");
        assert!(
            prop.passed,
            "addr, roll, and warp should coexist without interference"
        );
    }

    #[test]
    #[serial]
    fn addr_corpus_isolation() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAddr.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let store_sel: [u8; 4] = [0x82, 0xbd, 0xfc, 0xe9]; // call_store_pk_1()
        let calls_a = vec![Call {
            selector: store_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        let setup_only_sel: [u8; 4] = [0xd6, 0x3c, 0xad, 0xf0]; // property_setup_addr_persists()
        let calls_b = vec![Call {
            selector: setup_only_sel,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_b = chain.execute(&calls_b).unwrap();
        assert!(output_b.all_ok, "sequence B should succeed");
        let prop = output_b
            .property_results
            .iter()
            .find(|p| p.name == "property_setup_addr_persists")
            .expect("property should exist");
        assert!(
            prop.passed,
            "stored address from sequence A must not leak into sequence B"
        );
    }
}
