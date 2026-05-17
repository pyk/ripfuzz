//! `fee` cheatcode — set and persist `block.basefee`.

use revm::primitives::{Bytes, U256};

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect, decode_u256_arg};

pub struct Fee;

impl Cheatcode for Fee {
    type Args = U256;
    const SELECTOR: [u8; 4] = [0x39, 0xb3, 0x7a, 0xb0];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_u256_arg(input)
    }

    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        // Match Foundry: reject values that do not fit in u64.
        match u64::try_from(value) {
            Ok(basefee) => vec![CheatcodeEffect::SetBaseFee(basefee)],
            Err(_) => vec![CheatcodeEffect::Revert(
                "fee: base fee exceeds u64::MAX".into(),
            )],
        }
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
    fn fee_decode_and_effects() {
        let mut data = Fee::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(42u64).to_be_bytes_vec());
        let args = Fee::decode(&Bytes::from(data)).unwrap();
        let effects = Fee::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::SetBaseFee(42)]);
    }

    #[test]
    fn fee_decode_zero() {
        let mut data = Fee::SELECTOR.to_vec();
        data.extend_from_slice(&U256::ZERO.to_be_bytes_vec());
        let args = Fee::decode(&Bytes::from(data)).unwrap();
        let effects = Fee::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::SetBaseFee(0)]);
    }

    #[test]
    fn fee_decode_max_uint64() {
        let mut data = Fee::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(u64::MAX).to_be_bytes_vec());
        let args = Fee::decode(&Bytes::from(data)).unwrap();
        let effects = Fee::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::SetBaseFee(u64::MAX)]);
    }

    #[test]
    fn fee_decode_overflow_reverts() {
        let mut data = Fee::SELECTOR.to_vec();
        data.extend_from_slice(&(U256::from(u64::MAX) + U256::from(1)).to_be_bytes_vec());
        let args = Fee::decode(&Bytes::from(data)).unwrap();
        let effects = Fee::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::Revert(
                "fee: base fee exceeds u64::MAX".into(),
            )]
        );
    }

    #[test]
    fn cheatcode_fee_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeFee.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_record: [u8; 4] = [0xd3, 0x12, 0xbe, 0x09]; // action_record_basefee()
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
            .find(|p| p.name == "property_setup_fee_persists")
            .expect("property should exist");
        assert!(prop.passed, "setUp fee should persist into first call");
    }

    #[test]
    fn cheatcode_fee_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeFee.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_fee: [u8; 4] = [0xd7, 0x32, 0x98, 0xc4]; // action_fee(uint256)
        let action_record: [u8; 4] = [0xd3, 0x12, 0xbe, 0x09]; // action_record_basefee()
        let mut args = vec![0u8; 32];
        args[31] = 100; // U256(100)
        let calls = vec![
            Call {
                selector: action_fee,
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
            .find(|p| p.name == "property_fee_persists_across_calls")
            .expect("property should exist");
        assert!(
            prop.passed,
            "fee should persist across calls with no auto-advance"
        );
    }

    #[test]
    fn cheatcode_fee_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeFee.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_fee_revert: [u8; 4] = [0x4d, 0x01, 0x2a, 0x77]; // action_fee_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![Call {
            selector: action_fee_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "fee_and_revert should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_revert_undoes_fee")
            .expect("property should exist");
        assert!(prop.passed, "reverted fee should not leak into properties");
    }

    #[test]
    fn cheatcode_fee_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeFee.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_fee_100: [u8; 4] = [0xcf, 0xb8, 0x64, 0x4c]; // action_fee_100()
        let action_fee_200: [u8; 4] = [0x80, 0x4f, 0x8d, 0x17]; // action_fee_200()
        let action_record: [u8; 4] = [0xd3, 0x12, 0xbe, 0x09]; // action_record_basefee()
        let calls = vec![
            Call {
                selector: action_fee_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_fee_200,
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
            .find(|p| p.name == "property_fee_overwrite")
            .expect("property should exist");
        assert!(prop.passed, "fee overwrite should produce correct basefee");
    }

    #[test]
    fn cheatcode_fee_zero_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeFee.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_fee_zero: [u8; 4] = [0x5c, 0xb6, 0x38, 0x2e]; // action_fee_zero()
        let action_record: [u8; 4] = [0xd3, 0x12, 0xbe, 0x09]; // action_record_basefee()
        let calls = vec![
            Call {
                selector: action_fee_zero,
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
            .find(|p| p.name == "property_fee_zero")
            .expect("property should exist");
        assert!(prop.passed, "fee to zero should produce correct basefee");
    }

    #[test]
    fn cheatcode_fee_max_uint64_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeFee.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_fee_max: [u8; 4] = [0x7e, 0xcd, 0x2d, 0xa6]; // action_fee_max_uint64()
        let calls = vec![Call {
            selector: action_fee_max,
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
            .find(|p| p.name == "property_fee_max_uint64")
            .expect("property should exist");
        assert!(
            prop.passed,
            "fee to max uint64 should produce correct basefee"
        );
    }

    #[test]
    fn cheatcode_fee_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeFee.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_fee: [u8; 4] = [0xd7, 0x32, 0x98, 0xc4]; // action_fee(uint256)
        let action_record: [u8; 4] = [0xd3, 0x12, 0xbe, 0x09]; // action_record_basefee()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls_a = vec![Call {
            selector: action_fee,
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
            "fee from sequence A should not leak into sequence B"
        );
    }

    #[test]
    fn cheatcode_fee_property_final_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeFee.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_fee_100: [u8; 4] = [0xcf, 0xb8, 0x64, 0x4c]; // action_fee_100()
        let calls = vec![Call {
            selector: action_fee_100,
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
            .find(|p| p.name == "property_final_basefee")
            .expect("property should exist");
        assert!(prop.passed, "property should see the final fee basefee");
    }

    #[test]
    fn cheatcode_fee_roll_warp_interaction_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeFee.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_fee_and_roll_warp: [u8; 4] = [0xd2, 0x54, 0x3c, 0xd0]; // action_fee_and_roll_warp()
        let calls = vec![Call {
            selector: action_fee_and_roll_warp,
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
            .find(|p| p.name == "property_fee_and_roll_warp")
            .expect("property should exist");
        assert!(
            prop.passed,
            "fee, roll, and warp should coexist without interference"
        );
    }
}
