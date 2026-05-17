//! `prevrandao` cheatcode — set and persist `block.prevrandao` on a post-Paris
//! chain.
//!
//! Raptor follows the Foundry / Echidna persistent model: a `prevrandao`
//! mutation committed during a call remains visible for the rest of the
//! sequence (and for property checks) until the `ChainState` clone is
//! discarded.  This is consistent with how `warp`, `roll`, `fee`, and
//! `coinbase` already behave in raptor.

use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

pub struct Prevrandao;

impl Cheatcode for Prevrandao {
    type Args = [u8; 32];
    const SELECTOR: [u8; 4] = [0x3b, 0x92, 0x55, 0x49];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 + 32 {
            return None;
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&input[4..4 + 32]);
        Some(bytes)
    }

    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetPrevrandao(value)]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::primitives::Bytes;
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    // ------------------------------------------------------------------
    // Unit tests
    // ------------------------------------------------------------------

    #[test]
    fn prevrandao_decode_and_effects() {
        let mut data = Prevrandao::SELECTOR.to_vec();
        let arg = [0xca; 32];
        data.extend_from_slice(&arg);
        let args = Prevrandao::decode(&Bytes::from(data)).unwrap();
        assert_eq!(
            Prevrandao::effects(args),
            vec![CheatcodeEffect::SetPrevrandao(arg)]
        );
    }

    #[test]
    fn prevrandao_decode_zero() {
        let mut data = Prevrandao::SELECTOR.to_vec();
        let arg = [0u8; 32];
        data.extend_from_slice(&arg);
        let args = Prevrandao::decode(&Bytes::from(data)).unwrap();
        assert_eq!(
            Prevrandao::effects(args),
            vec![CheatcodeEffect::SetPrevrandao(arg)]
        );
    }

    #[test]
    fn prevrandao_decode_max() {
        let mut data = Prevrandao::SELECTOR.to_vec();
        let arg = [0xff; 32];
        data.extend_from_slice(&arg);
        let args = Prevrandao::decode(&Bytes::from(data)).unwrap();
        assert_eq!(
            Prevrandao::effects(args),
            vec![CheatcodeEffect::SetPrevrandao(arg)]
        );
    }

    #[test]
    fn prevrandao_decode_too_short() {
        let mut data = Prevrandao::SELECTOR.to_vec();
        data.extend_from_slice(&[0u8; 31]); // one byte short
        let args = Prevrandao::decode(&Bytes::from(data));
        assert!(args.is_none());
    }

    #[test]
    fn prevrandao_decode_selector_only() {
        let data = Prevrandao::SELECTOR.to_vec();
        let args = Prevrandao::decode(&Bytes::from(data));
        assert!(args.is_none());
    }

    // ------------------------------------------------------------------
    // Integration tests
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn cheatcode_prevrandao_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_record: [u8; 4] = [0x49, 0xfc, 0xf8, 0x23]; // action_record_prevrandao()
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
            .find(|p| p.name == "property_setup_prevrandao_persists")
            .expect("property should exist");
        assert!(
            prop.passed,
            "setUp prevrandao should persist into first call"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_prevrandao: [u8; 4] = [0x54, 0x0b, 0xa4, 0x8b]; // action_prevrandao(bytes32)
        let action_record: [u8; 4] = [0x49, 0xfc, 0xf8, 0x23]; // action_record_prevrandao()
        let mut args = vec![0u8; 32];
        args[31] = 0xAB;
        let calls = vec![
            Call {
                selector: action_prevrandao,
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
            .find(|p| p.name == "property_prevrandao_persists_across_calls")
            .expect("property should exist");
        assert!(
            prop.passed,
            "prevrandao should persist across calls with no auto-advance"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_revert: [u8; 4] = [0x44, 0x5d, 0xf8, 0x84]; // action_prevrandao_and_revert(bytes32)
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
        assert!(!output.all_ok, "prevrandao_and_revert should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_revert_undoes_prevrandao")
            .expect("property should exist");
        assert!(
            prop.passed,
            "reverted prevrandao should not leak into properties"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_a: [u8; 4] = [0xfb, 0xba, 0xce, 0x4f]; // action_prevrandao_A()
        let action_b: [u8; 4] = [0x1f, 0x92, 0x86, 0xa3]; // action_prevrandao_B()
        let action_record: [u8; 4] = [0x49, 0xfc, 0xf8, 0x23]; // action_record_prevrandao()
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
            .find(|p| p.name == "property_prevrandao_overwrite")
            .expect("property should exist");
        assert!(
            prop.passed,
            "prevrandao overwrite should produce correct value"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_zero_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_zero: [u8; 4] = [0x8e, 0xb1, 0xb2, 0x52]; // action_prevrandao_zero()
        let action_record: [u8; 4] = [0x49, 0xfc, 0xf8, 0x23]; // action_record_prevrandao()
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
            .find(|p| p.name == "property_prevrandao_zero")
            .expect("property should exist");
        assert!(
            prop.passed,
            "prevrandao to zero should produce correct value"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_max_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_max: [u8; 4] = [0x95, 0x5e, 0x40, 0x20]; // action_prevrandao_max()
        let calls = vec![Call {
            selector: action_max,
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
            .find(|p| p.name == "property_prevrandao_max")
            .expect("property should exist");
        assert!(
            prop.passed,
            "prevrandao to max should produce correct value"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_prevrandao: [u8; 4] = [0x54, 0x0b, 0xa4, 0x8b]; // action_prevrandao(bytes32)
        let action_record: [u8; 4] = [0x49, 0xfc, 0xf8, 0x23]; // action_record_prevrandao()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xDEADu32.to_be_bytes());
        let calls_a = vec![Call {
            selector: action_prevrandao,
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
            "prevrandao from sequence A should not leak into sequence B"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_property_final_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_a: [u8; 4] = [0xfb, 0xba, 0xce, 0x4f]; // action_prevrandao_A()
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
            .find(|p| p.name == "property_final_prevrandao")
            .expect("property should exist");
        assert!(
            prop.passed,
            "property should see the final prevrandao value"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_roll_warp_fee_coinbase_interaction_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_interaction: [u8; 4] = [0x1c, 0xa5, 0x55, 0x47]; // action_prevrandao_and_roll_warp_fee_coinbase()
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
            .find(|p| p.name == "property_prevrandao_and_roll_warp_fee_coinbase")
            .expect("property should exist");
        assert!(
            prop.passed,
            "prevrandao, roll, warp, fee, and coinbase should coexist without interference"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_difficulty_interaction_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_interaction: [u8; 4] = [0xdf, 0x9f, 0xed, 0x5d]; // action_prevrandao_then_difficulty()
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
            .find(|p| p.name == "property_difficulty_noop_does_not_clobber")
            .expect("property should exist");
        assert!(
            prop.passed,
            "difficulty no-op should not clobber a prior prevrandao setting"
        );
    }
}
