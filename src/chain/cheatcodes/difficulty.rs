//! `difficulty` cheatcode — no-op on raptor's post-Paris default chain.
//!
//! After the Paris (Merge) hard fork, `block.difficulty` was deprecated and
//! replaced by `block.prevrandao` (EIP-4399). On a post-Paris chain the
//! `DIFFICULTY` opcode reads `prevrandao`, not the `difficulty` field of the
//! block header. Raptor uses `Context::mainnet()` which targets a post-Paris
//! specification, so `vm.difficulty(uint256)` cannot meaningfully mutate
//! execution context. The canonical behaviour across modern fuzzers is a no-op.
//! Users who want to control the value returned by the `DIFFICULTY` opcode
//! should use `vm.prevrandao(bytes32)` instead.

use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

pub struct Difficulty;

impl Cheatcode for Difficulty {
    type Args = ();
    const SELECTOR: [u8; 4] = [0x46, 0xcc, 0x92, 0xd9];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        // The selector is followed by one uint256 argument, but on a post-Paris
        // chain the value is ignored. We only validate that the calldata is
        // long enough to contain the argument.
        if input.len() < 4 + 32 {
            return None;
        }
        Some(())
    }

    fn effects(_args: Self::Args) -> Vec<CheatcodeEffect> {
        // Intentional no-op: raptor runs on a post-Paris chain where the
        // DIFFICULTY opcode reads prevrandao, not block.difficulty.
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::primitives::{Bytes, U256};
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    // ------------------------------------------------------------------
    // Unit tests
    // ------------------------------------------------------------------

    #[test]
    fn difficulty_decode_valid() {
        let mut data = Difficulty::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(42u64).to_be_bytes_vec());
        let args = Difficulty::decode(&Bytes::from(data));
        assert!(args.is_some());
        assert_eq!(Difficulty::effects(args.unwrap()), vec![]);
    }

    #[test]
    fn difficulty_decode_zero() {
        let mut data = Difficulty::SELECTOR.to_vec();
        data.extend_from_slice(&U256::ZERO.to_be_bytes_vec());
        let args = Difficulty::decode(&Bytes::from(data));
        assert!(args.is_some());
        assert_eq!(Difficulty::effects(args.unwrap()), vec![]);
    }

    #[test]
    fn difficulty_decode_max_uint256() {
        let mut data = Difficulty::SELECTOR.to_vec();
        data.extend_from_slice(&U256::MAX.to_be_bytes_vec());
        let args = Difficulty::decode(&Bytes::from(data));
        assert!(args.is_some());
        assert_eq!(Difficulty::effects(args.unwrap()), vec![]);
    }

    #[test]
    fn difficulty_decode_too_short() {
        let mut data = Difficulty::SELECTOR.to_vec();
        data.extend_from_slice(&[0u8; 31]); // one byte short
        let args = Difficulty::decode(&Bytes::from(data));
        assert!(args.is_none());
    }

    #[test]
    fn difficulty_decode_selector_only() {
        let data = Difficulty::SELECTOR.to_vec();
        let args = Difficulty::decode(&Bytes::from(data));
        assert!(args.is_none());
    }

    // ------------------------------------------------------------------
    // Integration tests
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn cheatcode_difficulty_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDifficulty.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_record: [u8; 4] = [0x60, 0x13, 0x31, 0x7c]; // action_record()
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
            .find(|p| p.name == "property_setup_difficulty_unchanged")
            .expect("property should exist");
        assert!(prop.passed, "setUp difficulty should be a no-op");
    }

    #[test]
    #[serial]
    fn cheatcode_difficulty_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDifficulty.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_difficulty: [u8; 4] = [0xaf, 0x13, 0x6f, 0x0d]; // action_difficulty(uint256)
        let action_record: [u8; 4] = [0x60, 0x13, 0x31, 0x7c]; // action_record()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&12345u32.to_be_bytes());
        let calls = vec![
            Call {
                selector: action_difficulty,
                args,
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
            .find(|p| p.name == "property_difficulty_no_op")
            .expect("property should exist");
        assert!(prop.passed, "difficulty should be a no-op in sequence");
    }

    #[test]
    #[serial]
    fn cheatcode_difficulty_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDifficulty.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_difficulty_and_revert: [u8; 4] = [0xfb, 0x82, 0x50, 0x87]; // action_difficulty_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![Call {
            selector: action_difficulty_and_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "difficulty_and_revert should revert");
        let prop = output
            .property_results
            .iter()
            .find(|p| p.name == "property_revert_is_still_no_op")
            .expect("property should exist");
        assert!(
            prop.passed,
            "reverted difficulty should not leak into properties"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_difficulty_prevrandao_interaction_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDifficulty.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_prevrandao_then_difficulty: [u8; 4] = [0xdf, 0x9f, 0xed, 0x5d]; // action_prevrandao_then_difficulty()
        let calls = vec![Call {
            selector: action_prevrandao_then_difficulty,
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
            .find(|p| p.name == "property_prevrandao_unaffected")
            .expect("property should exist");
        assert!(
            prop.passed,
            "difficulty should not clobber a prior prevrandao setting"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_difficulty_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDifficulty.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_difficulty: [u8; 4] = [0xaf, 0x13, 0x6f, 0x0d]; // action_difficulty(uint256)
        let action_record: [u8; 4] = [0x60, 0x13, 0x31, 0x7c]; // action_record()
        let mut args1 = vec![0u8; 32];
        args1[31] = 1;
        let mut args2 = vec![0u8; 32];
        args2[31] = 2;
        let calls = vec![
            Call {
                selector: action_difficulty,
                args: args1,
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: action_difficulty,
                args: args2,
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
            .find(|p| p.name == "property_difficulty_still_unchanged")
            .expect("property should exist");
        assert!(
            prop.passed,
            "multiple difficulty calls should still be no-ops"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_difficulty_zero_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDifficulty.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_difficulty_zero: [u8; 4] = [0x88, 0x56, 0x7e, 0x2a]; // action_difficulty_zero()
        let action_record: [u8; 4] = [0x60, 0x13, 0x31, 0x7c]; // action_record()
        let calls = vec![
            Call {
                selector: action_difficulty_zero,
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
            .find(|p| p.name == "property_difficulty_zero_no_op")
            .expect("property should exist");
        assert!(prop.passed, "difficulty to zero should still be a no-op");
    }

    #[test]
    #[serial]
    fn cheatcode_difficulty_max_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDifficulty.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_difficulty_max: [u8; 4] = [0xc2, 0xbb, 0x84, 0x73]; // action_difficulty_max()
        let action_record: [u8; 4] = [0x60, 0x13, 0x31, 0x7c]; // action_record()
        let calls = vec![
            Call {
                selector: action_difficulty_max,
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
            .find(|p| p.name == "property_difficulty_max_no_op")
            .expect("property should exist");
        assert!(
            prop.passed,
            "difficulty to max uint64 should still be a no-op"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_difficulty_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDifficulty.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_difficulty: [u8; 4] = [0xaf, 0x13, 0x6f, 0x0d]; // action_difficulty(uint256)
        let action_record: [u8; 4] = [0x60, 0x13, 0x31, 0x7c]; // action_record()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls_a = vec![Call {
            selector: action_difficulty,
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
            "difficulty from sequence A should not leak into sequence B"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_difficulty_property_final_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeDifficulty.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_noop: [u8; 4] = [0x27, 0x47, 0xbc, 0x32]; // action_noop()
        let calls = vec![Call {
            selector: action_noop,
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
            .find(|p| p.name == "property_final_difficulty")
            .expect("property should exist");
        assert!(prop.passed, "property should see the default difficulty");
    }
}
