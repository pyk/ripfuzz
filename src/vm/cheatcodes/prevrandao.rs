//! `prevrandao` cheatcode — set and persist `block.prevrandao` on a post-Paris
//! chain.
//!
//! Raptor follows the Foundry / Echidna persistent model: a `prevrandao`
//! mutation committed during a call remains visible for the rest of the
//! sequence (and for invariant checks) until the `ChainState` clone is
//! discarded.  This is consistent with how `warp`, `roll`, `fee`, and
//! `coinbase` already behave in raptor.

use revm::primitives::Bytes;

use crate::vm::{Cheatcode, CheatcodeEffect};

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

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0xdb, 0xf0, 0x3b, 0x83]; // call_record_prevrandao()
        let calls = vec![Call {
            selector: call_record,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_prevrandao: [u8; 4] = [0xfc, 0xfe, 0xad, 0xd3]; // call_prevrandao(bytes32)
        let call_record: [u8; 4] = [0xdb, 0xf0, 0x3b, 0x83]; // call_record_prevrandao()
        let mut args = vec![0u8; 32];
        args[31] = 0xAB;
        let calls = vec![
            Call {
                selector: call_prevrandao,
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
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_revert: [u8; 4] = [0x60, 0xb1, 0x2d, 0x9d]; // call_prevrandao_and_revert(bytes32)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xDEADu32.to_be_bytes());
        let calls = vec![Call {
            selector: call_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "prevrandao_and_revert should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_a: [u8; 4] = [0x05, 0xc3, 0xdb, 0x72]; // call_prevrandao_A()
        let call_b: [u8; 4] = [0xb6, 0xb1, 0xa3, 0x32]; // call_prevrandao_B()
        let call_record: [u8; 4] = [0xdb, 0xf0, 0x3b, 0x83]; // call_record_prevrandao()
        let calls = vec![
            Call {
                selector: call_a,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_b,
                args: vec![],
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
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_zero_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_zero: [u8; 4] = [0x4e, 0x48, 0x69, 0xb8]; // call_prevrandao_zero()
        let call_record: [u8; 4] = [0xdb, 0xf0, 0x3b, 0x83]; // call_record_prevrandao()
        let calls = vec![
            Call {
                selector: call_zero,
                args: vec![],
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
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_max_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_max: [u8; 4] = [0x9f, 0x00, 0x89, 0xb9]; // call_prevrandao_max()
        let calls = vec![Call {
            selector: call_max,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_prevrandao: [u8; 4] = [0xfc, 0xfe, 0xad, 0xd3]; // call_prevrandao(bytes32)
        let call_record: [u8; 4] = [0xdb, 0xf0, 0x3b, 0x83]; // call_record_prevrandao()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xDEADu32.to_be_bytes());
        let calls_a = vec![Call {
            selector: call_prevrandao,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        let calls_b = vec![Call {
            selector: call_record,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_b = chain.execute(&calls_b).unwrap();
        assert!(output_b.all_ok, "sequence B should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_invariant_final_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_a: [u8; 4] = [0x05, 0xc3, 0xdb, 0x72]; // call_prevrandao_A()
        let calls = vec![Call {
            selector: call_a,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_roll_warp_fee_coinbase_interaction_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_interaction: [u8; 4] = [0x24, 0xed, 0x68, 0x04]; // call_prevrandao_and_roll_warp_fee_coinbase()
        let calls = vec![Call {
            selector: call_interaction,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_prevrandao_difficulty_interaction_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrevrandao.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_interaction: [u8; 4] = [0x12, 0xd5, 0x74, 0x98]; // call_prevrandao_then_difficulty()
        let calls = vec![Call {
            selector: call_interaction,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }
}
