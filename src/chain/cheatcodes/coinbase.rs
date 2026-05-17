//! `coinbase` cheatcode — set and persist `block.coinbase`.

use revm::primitives::{Address, Bytes};

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect, decode_address_arg};

pub struct Coinbase;

impl Cheatcode for Coinbase {
    type Args = Address;
    const SELECTOR: [u8; 4] = [0xff, 0x48, 0x3c, 0x54];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_arg(input)
    }

    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetBeneficiary(value)]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::primitives::{Address, Bytes};
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn coinbase_decode_and_effects() {
        let addr = Address::new([0xca; 20]);
        let mut data = Coinbase::SELECTOR.to_vec();
        let mut padded = vec![0u8; 32];
        padded[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded);
        let args = Coinbase::decode(&Bytes::from(data)).unwrap();
        assert_eq!(
            Coinbase::effects(args),
            vec![CheatcodeEffect::SetBeneficiary(addr)]
        );
    }

    #[test]
    fn coinbase_decode_zero_address() {
        let addr = Address::ZERO;
        let mut data = Coinbase::SELECTOR.to_vec();
        let mut padded = vec![0u8; 32];
        padded[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded);
        let args = Coinbase::decode(&Bytes::from(data)).unwrap();
        assert_eq!(
            Coinbase::effects(args),
            vec![CheatcodeEffect::SetBeneficiary(Address::ZERO)]
        );
    }

    #[test]
    #[serial]
    fn cheatcode_coinbase_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeCoinbase.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_record: [u8; 4] = [0x5a, 0xfe, 0xc4, 0x44]; // action_record_coinbase()
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
            .find(|p| p.name == "property_setup_coinbase_persists")
            .expect("property should exist");
        assert!(prop.passed, "setUp coinbase should persist into first call");
    }

    #[test]
    #[serial]
    fn cheatcode_coinbase_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeCoinbase.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_coinbase: [u8; 4] = [0x6c, 0x22, 0x93, 0x18]; // action_coinbase(address)
        let action_record: [u8; 4] = [0x5a, 0xfe, 0xc4, 0x44]; // action_record_coinbase()
        let mut args = vec![0u8; 32];
        args[31] = 0xAB;
        let calls = vec![
            Call {
                selector: action_coinbase,
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
            .find(|p| p.name == "property_coinbase_persists_across_calls")
            .expect("property should exist");
        assert!(
            prop.passed,
            "coinbase should persist across calls with no auto-advance"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_coinbase_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeCoinbase.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_revert: [u8; 4] = [0x95, 0xd0, 0xd1, 0xe5]; // action_coinbase_and_revert(address)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xDEADu32.to_be_bytes());
        let calls = vec![Call {
            selector: action_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "coinbase_and_revert should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_revert_undoes_coinbase")
            .expect("property should exist");
        assert!(
            prop.passed,
            "reverted coinbase should not leak into properties"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_coinbase_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeCoinbase.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_a: [u8; 4] = [0xc1, 0x4a, 0xd2, 0xe7]; // action_coinbase_A()
        let action_b: [u8; 4] = [0x53, 0xe3, 0xc8, 0x59]; // action_coinbase_B()
        let action_record: [u8; 4] = [0x5a, 0xfe, 0xc4, 0x44]; // action_record_coinbase()
        let calls = vec![
            Call {
                selector: action_a,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_b,
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
            .find(|p| p.name == "property_coinbase_overwrite")
            .expect("property should exist");
        assert!(
            prop.passed,
            "coinbase overwrite should produce correct beneficiary"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_coinbase_zero_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeCoinbase.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_zero: [u8; 4] = [0x2e, 0x24, 0x88, 0x00]; // action_coinbase_zero()
        let action_record: [u8; 4] = [0x5a, 0xfe, 0xc4, 0x44]; // action_record_coinbase()
        let calls = vec![
            Call {
                selector: action_zero,
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
            .find(|p| p.name == "property_coinbase_zero")
            .expect("property should exist");
        assert!(
            prop.passed,
            "coinbase to zero should produce correct beneficiary"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_coinbase_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeCoinbase.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_coinbase: [u8; 4] = [0x6c, 0x22, 0x93, 0x18]; // action_coinbase(address)
        let action_record: [u8; 4] = [0x5a, 0xfe, 0xc4, 0x44]; // action_record_coinbase()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xDEADu32.to_be_bytes());
        let calls_a = vec![Call {
            selector: action_coinbase,
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
            "coinbase from sequence A should not leak into sequence B"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_coinbase_property_final_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeCoinbase.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_a: [u8; 4] = [0xc1, 0x4a, 0xd2, 0xe7]; // action_coinbase_A()
        let calls = vec![Call {
            selector: action_a,
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
            .find(|p| p.name == "property_final_coinbase")
            .expect("property should exist");
        assert!(prop.passed, "property should see the final coinbase value");
    }

    #[test]
    #[serial]
    fn cheatcode_coinbase_roll_warp_fee_interaction_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeCoinbase.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_interaction: [u8; 4] = [0xf3, 0x59, 0x4d, 0xda]; // action_coinbase_and_roll_warp_fee()
        let calls = vec![Call {
            selector: action_interaction,
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
            .find(|p| p.name == "property_coinbase_and_roll_warp_fee")
            .expect("property should exist");
        assert!(
            prop.passed,
            "coinbase, roll, warp, and fee should coexist without interference"
        );
    }
}
