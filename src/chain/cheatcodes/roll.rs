//! `roll` cheatcode — set and persist `block.number`.

use revm::primitives::{Bytes, U256};

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect, decode_u256_arg};

pub struct Roll;

impl Cheatcode for Roll {
    type Args = U256;
    const SELECTOR: [u8; 4] = [0x1f, 0x7b, 0x4f, 0x30];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_u256_arg(input)
    }

    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetBlockNumber(value)]
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
    fn roll_decode_and_effects() {
        let mut data = Roll::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(42u64).to_be_bytes_vec());
        let args = Roll::decode(&Bytes::from(data)).unwrap();
        let effects = Roll::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetBlockNumber(U256::from(42u64))]
        );
    }

    #[test]
    fn roll_decode_zero() {
        let mut data = Roll::SELECTOR.to_vec();
        data.extend_from_slice(&U256::ZERO.to_be_bytes_vec());
        let args = Roll::decode(&Bytes::from(data)).unwrap();
        let effects = Roll::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::SetBlockNumber(U256::ZERO)]);
    }

    #[test]
    fn roll_decode_max_uint64() {
        let mut data = Roll::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(u64::MAX).to_be_bytes_vec());
        let args = Roll::decode(&Bytes::from(data)).unwrap();
        let effects = Roll::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetBlockNumber(U256::from(u64::MAX))]
        );
    }

    #[test]
    fn cheatcode_roll_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_record: [u8; 4] = [0xca, 0x74, 0x7d, 0x75]; // action_record_block_number()
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
            .find(|p| p.name == "property_setup_roll_persists")
            .expect("property should exist");
        assert!(prop.passed, "setUp roll should persist into first call");
    }

    #[test]
    fn cheatcode_roll_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_roll: [u8; 4] = [0xd2, 0x2a, 0xdc, 0x8c]; // action_roll(uint256)
        let action_record: [u8; 4] = [0xca, 0x74, 0x7d, 0x75]; // action_record_block_number()
        let mut args = vec![0u8; 32];
        args[31] = 100; // U256(100)
        let calls = vec![
            Call {
                selector: action_roll,
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
            .find(|p| p.name == "property_roll_persists_across_calls")
            .expect("property should exist");
        assert!(
            prop.passed,
            "roll should persist across calls with min delay"
        );
    }

    #[test]
    fn cheatcode_roll_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_roll_revert: [u8; 4] = [0xb8, 0xd8, 0x40, 0xe9]; // action_roll_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![Call {
            selector: action_roll_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "roll_and_revert should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_revert_undoes_roll")
            .expect("property should exist");
        assert!(prop.passed, "reverted roll should not leak into properties");
    }

    #[test]
    fn cheatcode_roll_delay_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_roll_100: [u8; 4] = [0xc0, 0x05, 0xfd, 0x27]; // action_roll_100()
        let action_record: [u8; 4] = [0xca, 0x74, 0x7d, 0x75]; // action_record_block_number()
        let calls = vec![
            Call {
                selector: action_roll_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_record,
                args: vec![],
                block_number_delay: 5,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "actions should succeed");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_roll_with_delay")
            .expect("property should exist");
        assert!(
            prop.passed,
            "roll with delay should produce correct block number"
        );
    }

    #[test]
    fn cheatcode_roll_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_roll_100: [u8; 4] = [0xc0, 0x05, 0xfd, 0x27]; // action_roll_100()
        let action_roll_200: [u8; 4] = [0xce, 0x5a, 0x81, 0x98]; // action_roll_200()
        let action_record: [u8; 4] = [0xca, 0x74, 0x7d, 0x75]; // action_record_block_number()
        let calls = vec![
            Call {
                selector: action_roll_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_roll_200,
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
            .find(|p| p.name == "property_roll_overwrite")
            .expect("property should exist");
        assert!(
            prop.passed,
            "roll overwrite should produce correct block number"
        );
    }

    #[test]
    fn cheatcode_roll_zero_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_roll_zero: [u8; 4] = [0x9d, 0x26, 0x9c, 0x17]; // action_roll_zero()
        let action_record: [u8; 4] = [0xca, 0x74, 0x7d, 0x75]; // action_record_block_number()
        let calls = vec![
            Call {
                selector: action_roll_zero,
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
            .find(|p| p.name == "property_roll_zero")
            .expect("property should exist");
        assert!(
            prop.passed,
            "roll to zero should produce correct block number"
        );
    }

    #[test]
    fn cheatcode_roll_max_uint64_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_roll_max: [u8; 4] = [0x38, 0x45, 0x06, 0xe9]; // action_roll_max_uint64()
        let calls = vec![Call {
            selector: action_roll_max,
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
            .find(|p| p.name == "property_roll_max_uint64")
            .expect("property should exist");
        assert!(
            prop.passed,
            "roll to max uint64 should produce correct block number"
        );
    }

    #[test]
    fn cheatcode_roll_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_roll: [u8; 4] = [0xd2, 0x2a, 0xdc, 0x8c]; // action_roll(uint256)
        let action_record: [u8; 4] = [0xca, 0x74, 0x7d, 0x75]; // action_record_block_number()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls_a = vec![Call {
            selector: action_roll,
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
            "roll from sequence A should not leak into sequence B"
        );
    }

    #[test]
    fn cheatcode_roll_property_final_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_roll_100: [u8; 4] = [0xc0, 0x05, 0xfd, 0x27]; // action_roll_100()
        let calls = vec![Call {
            selector: action_roll_100,
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
            .find(|p| p.name == "property_final_block_number")
            .expect("property should exist");
        assert!(
            prop.passed,
            "property should see the final rolled block number"
        );
    }

    #[test]
    fn cheatcode_roll_warp_interaction_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeRoll.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_roll_and_warp: [u8; 4] = [0xf9, 0xe3, 0x4d, 0x7e]; // action_roll_and_warp()
        let calls = vec![Call {
            selector: action_roll_and_warp,
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
            .find(|p| p.name == "property_roll_and_warp")
            .expect("property should exist");
        assert!(
            prop.passed,
            "roll and warp should coexist without interference"
        );
    }
}
