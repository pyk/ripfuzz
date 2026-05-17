//! Balance manipulation cheatcode.

use revm::primitives::{Address, Bytes, U256};

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect, decode_address_u256_args};

pub struct Deal;

impl Cheatcode for Deal {
    type Args = (Address, U256);
    const SELECTOR: [u8; 4] = [0xc8, 0x8a, 0x5e, 0x6d];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_u256_args(input)
    }

    fn effects((addr, value): Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetAccountBalance(addr, value)]
    }
}

/// Snapshot of an address balance before `deal` was applied.
/// Required for rollback on revert (Foundry-compatible semantics).
#[derive(Clone, Debug, PartialEq)]
pub struct DealRecord {
    pub address: Address,
    pub old_balance: U256,
    pub new_balance: U256,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::{
        MainContext,
        context::Context,
        context_interface::{ContextTr, JournalTr},
        database::InMemoryDB,
        primitives::{Address, U256},
    };
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::effect::apply_effect;
    use crate::chain::inspectors::cheatcode::CheatcodeInspector;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn deal_decode_and_effects() {
        let mut data = Deal::SELECTOR.to_vec();
        let addr = Address::new([0xab; 20]);
        let value = U256::from(5_000u64);
        let mut padded_addr = vec![0u8; 32];
        padded_addr[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded_addr);
        data.extend_from_slice(&value.to_be_bytes_vec());
        let args = Deal::decode(&Bytes::from(data)).unwrap();
        let effects = Deal::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetAccountBalance(addr, value)]
        );
    }

    #[test]
    fn deal_effect_applies() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xab; 20]);
        let value = U256::from(5_000u64);
        let effects = Deal::effects((addr, value));
        for e in &effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        let info = ctx.journal_mut().load_account(addr).unwrap().data;
        assert_eq!(info.info.balance, value);
        assert_eq!(
            inspector.state.eth_deals,
            vec![DealRecord {
                address: addr,
                old_balance: U256::ZERO,
                new_balance: value,
            }]
        );
    }

    #[test]
    #[serial]
    fn cheatcode_deal_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_record: [u8; 4] = [0x1f, 0xec, 0xba, 0xb3]; // call_record_target_balance()
        let calls = vec![Call {
            selector: call_record,
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
            .find(|p| p.name == "property_setup_deal_persists")
            .expect("property should exist");
        assert!(prop.passed, "setUp deal should persist into first call");
    }

    #[test]
    #[serial]
    fn cheatcode_deal_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_deal: [u8; 4] = [0xc3, 0x44, 0x3e, 0x79]; // call_deal(uint256)
        let call_record: [u8; 4] = [0x1f, 0xec, 0xba, 0xb3]; // call_record_target_balance()
        let mut args = vec![0u8; 32];
        args[31] = 100; // U256(100)
        let calls = vec![
            Call {
                selector: call_deal,
                args: args.clone(),
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_record,
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
            .find(|p| p.name == "property_deal_persists_across_calls")
            .expect("property should exist");
        assert!(
            prop.passed,
            "deal should persist across calls with no auto-advance"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_deal_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_deal_revert: [u8; 4] = [0x20, 0xcf, 0x7c, 0x0b]; // call_deal_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![Call {
            selector: call_deal_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "deal_and_revert should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_revert_undoes_deal")
            .expect("property should exist");
        assert!(prop.passed, "reverted deal should not leak into properties");
    }

    #[test]
    #[serial]
    fn cheatcode_deal_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_deal_100: [u8; 4] = [0x1d, 0xa2, 0x66, 0x31]; // call_deal_100()
        let call_deal_200: [u8; 4] = [0xe8, 0x37, 0xf1, 0x89]; // call_deal_200()
        let calls = vec![
            Call {
                selector: call_deal_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_deal_200,
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
            .find(|p| p.name == "property_deal_overwrite")
            .expect("property should exist");
        assert!(prop.passed, "deal overwrite should produce correct balance");
    }

    #[test]
    #[serial]
    fn cheatcode_deal_zero_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_deal_zero: [u8; 4] = [0x1c, 0x29, 0x90, 0xc9]; // call_deal_zero()
        let calls = vec![Call {
            selector: call_deal_zero,
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
            .find(|p| p.name == "property_deal_zero")
            .expect("property should exist");
        assert!(prop.passed, "deal to zero should produce correct balance");
    }

    #[test]
    #[serial]
    fn cheatcode_deal_max_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_deal_max: [u8; 4] = [0x10, 0x7d, 0x54, 0x7e]; // call_deal_max()
        let calls = vec![Call {
            selector: call_deal_max,
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
            .find(|p| p.name == "property_deal_max")
            .expect("property should exist");
        assert!(
            prop.passed,
            "deal to max uint256 should produce correct balance"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_deal_empty_address_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_deal_empty: [u8; 4] = [0x18, 0xfd, 0x7c, 0x84]; // call_deal_empty(uint256)
        let mut args = vec![0u8; 32];
        args[31] = 42; // U256(42)
        let calls = vec![Call {
            selector: call_deal_empty,
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
            .find(|p| p.name == "property_deal_empty")
            .expect("property should exist");
        assert!(
            prop.passed,
            "deal to empty address should produce correct balance"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_deal_property_final_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_deal_100: [u8; 4] = [0x1d, 0xa2, 0x66, 0x31]; // call_deal_100()
        let calls = vec![Call {
            selector: call_deal_100,
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
            .find(|p| p.name == "property_final_balance")
            .expect("property should exist");
        assert!(prop.passed, "property should see the final dealt balance");
    }

    #[test]
    #[serial]
    fn cheatcode_deal_cross_cheatcode_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_deal_and_warp_roll: [u8; 4] = [0x94, 0xd9, 0xd2, 0xbc]; // call_deal_and_warp_roll()
        let calls = vec![Call {
            selector: call_deal_and_warp_roll,
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
            .find(|p| p.name == "property_deal_and_warp_roll")
            .expect("property should exist");
        assert!(
            prop.passed,
            "deal, roll, and warp should coexist without interference"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_deal_self_overwrite_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_self_deal: [u8; 4] = [0xc4, 0xc7, 0xde, 0xea]; // call_self_deal(uint256)
        let mut args = vec![0u8; 32];
        args[24..32].copy_from_slice(&(1_000_000_000_000_000_000u64).to_be_bytes());
        let calls = vec![Call {
            selector: call_self_deal,
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
            .find(|p| p.name == "property_self_deal_overwrites_setup")
            .expect("property should exist");
        assert!(prop.passed, "self-deal should overwrite the setUp balance");
    }

    #[test]
    #[serial]
    fn cheatcode_deal_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDeal.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_deal_100: [u8; 4] = [0x1d, 0xa2, 0x66, 0x31]; // call_deal_100()
        let calls_a = vec![Call {
            selector: call_deal_100,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        let call_setup_only: [u8; 4] = [0x28, 0x98, 0x65, 0x69]; // property_setup_only()
        let calls_b = vec![Call {
            selector: call_setup_only,
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
            "deal from sequence A should not leak into sequence B"
        );
    }
}
