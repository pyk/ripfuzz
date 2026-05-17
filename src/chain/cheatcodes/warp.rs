//! `warp` cheatcode — set and persist `block.timestamp`.

use revm::primitives::{Bytes, U256};

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect, decode_u256_arg};

pub struct Warp;

impl Cheatcode for Warp {
    type Args = U256;
    const SELECTOR: [u8; 4] = [0xe5, 0xd6, 0xbf, 0x02];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_u256_arg(input)
    }

    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetBlockTimestamp(value)]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::primitives::U256;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn warp_decode_and_effects() {
        let mut data = Warp::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(1234567890u64).to_be_bytes_vec());
        let args = Warp::decode(&Bytes::from(data)).unwrap();
        let effects = Warp::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetBlockTimestamp(U256::from(
                1234567890u64
            ))]
        );
    }

    #[test]
    fn cheatcode_warp_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_record: [u8; 4] = [0x3c, 0x93, 0x0e, 0xf1]; // action_record_timestamp()
        let calls = vec![Call {
            selector: action_record,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "action should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_setup_warp_persists")
            .expect("property should exist");
        assert!(prop.passed, "setUp warp should persist into first call");
    }

    #[test]
    fn cheatcode_warp_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_warp: [u8; 4] = [0xa1, 0x65, 0xf3, 0x2a]; // action_warp(uint256)
        let action_record: [u8; 4] = [0x3c, 0x93, 0x0e, 0xf1]; // action_record_timestamp()
        let mut args = vec![0u8; 32];
        args[31] = 100; // U256(100)
        let calls = vec![
            Call {
                selector: action_warp,
                args: args.clone(),
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_record,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "actions should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_warp_persists_across_calls")
            .expect("property should exist");
        assert!(
            prop.passed,
            "warp should persist across calls with min delay"
        );
    }

    #[test]
    fn cheatcode_warp_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_warp_revert: [u8; 4] = [0x14, 0x16, 0x9d, 0x70]; // action_warp_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![Call {
            selector: action_warp_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "warp_and_revert should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_revert_undoes_warp")
            .expect("property should exist");
        assert!(prop.passed, "reverted warp should not leak into properties");
    }

    #[test]
    fn cheatcode_warp_delay_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_warp_100: [u8; 4] = [0xae, 0x19, 0xc7, 0xe5]; // action_warp_100()
        let action_record: [u8; 4] = [0x3c, 0x93, 0x0e, 0xf1]; // action_record_timestamp()
        let calls = vec![
            Call {
                selector: action_warp_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_record,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 5,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "actions should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_warp_with_delay")
            .expect("property should exist");
        assert!(
            prop.passed,
            "warp with delay should produce correct timestamp"
        );
    }

    #[test]
    fn cheatcode_warp_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_warp_100: [u8; 4] = [0xae, 0x19, 0xc7, 0xe5]; // action_warp_100()
        let action_warp_200: [u8; 4] = [0x95, 0x30, 0x12, 0x61]; // action_warp_200()
        let action_record: [u8; 4] = [0x3c, 0x93, 0x0e, 0xf1]; // action_record_timestamp()
        let calls = vec![
            Call {
                selector: action_warp_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_warp_200,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_record,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "actions should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_warp_overwrite")
            .expect("property should exist");
        assert!(
            prop.passed,
            "warp overwrite should produce correct timestamp"
        );
    }

    #[test]
    fn warp_decode_zero() {
        let mut data = Warp::SELECTOR.to_vec();
        data.extend_from_slice(&U256::ZERO.to_be_bytes_vec());
        let args = Warp::decode(&Bytes::from(data)).unwrap();
        let effects = Warp::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetBlockTimestamp(U256::ZERO)]
        );
    }

    #[test]
    fn warp_decode_max_uint64() {
        let mut data = Warp::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(u64::MAX).to_be_bytes_vec());
        let args = Warp::decode(&Bytes::from(data)).unwrap();
        let effects = Warp::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetBlockTimestamp(U256::from(u64::MAX))]
        );
    }

    #[test]
    fn cheatcode_warp_zero_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_warp_zero: [u8; 4] = [0xc0, 0x07, 0xfd, 0xe7];
        let action_record: [u8; 4] = [0x3c, 0x93, 0x0e, 0xf1];
        let calls = vec![
            Call {
                selector: action_warp_zero,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_record,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "actions should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_warp_zero")
            .expect("property should exist");
        assert!(prop.passed, "warp to zero should produce correct timestamp");
    }

    #[test]
    fn cheatcode_warp_max_uint64_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_warp_max: [u8; 4] = [0xaa, 0x59, 0x66, 0x2a];
        let calls = vec![Call {
            selector: action_warp_max,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "action should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_warp_max_uint64")
            .expect("property should exist");
        assert!(
            prop.passed,
            "warp to max uint64 should produce correct timestamp"
        );
    }

    #[test]
    fn cheatcode_warp_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_warp: [u8; 4] = [0xa1, 0x65, 0xf3, 0x2a];
        let action_record: [u8; 4] = [0x3c, 0x93, 0x0e, 0xf1];
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls_a = vec![Call {
            selector: action_warp,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        let calls_b = vec![Call {
            selector: action_record,
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
            .find(|p| p.name == "property_setup_only")
            .expect("property should exist");
        assert!(
            prop.passed,
            "warp from sequence A should not leak into sequence B"
        );
    }

    #[test]
    fn cheatcode_warp_property_final_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_warp_100: [u8; 4] = [0xae, 0x19, 0xc7, 0xe5];
        let calls = vec![Call {
            selector: action_warp_100,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "action should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_final_timestamp")
            .expect("property should exist");
        assert!(
            prop.passed,
            "property should see the final warped timestamp"
        );
    }

    #[test]
    fn cheatcode_warp_roll_interaction_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeWarp.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_warp_and_roll: [u8; 4] = [0x3b, 0xf2, 0x2f, 0x77];
        let calls = vec![Call {
            selector: action_warp_and_roll,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "action should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_warp_and_roll")
            .expect("property should exist");
        assert!(
            prop.passed,
            "warp and roll should coexist without interference"
        );
    }
}
