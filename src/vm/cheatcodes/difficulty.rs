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

use crate::vm::{Cheatcode, CheatcodeEffect};

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
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeDifficulty.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
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
    fn cheatcode_difficulty_sequence_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeDifficulty.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_difficulty: [u8; 4] = [0x59, 0x4a, 0x94, 0x30]; // call_difficulty(uint256)
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&12345u32.to_be_bytes());
        let calls = vec![
            Call {
                selector: call_difficulty,
                args,
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
    fn cheatcode_difficulty_revert_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeDifficulty.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_difficulty_and_revert: [u8; 4] = [0x60, 0xa9, 0x6a, 0x8e]; // call_difficulty_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![Call {
            selector: call_difficulty_and_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "difficulty_and_revert should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_difficulty_prevrandao_interaction_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeDifficulty.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_prevrandao_then_difficulty: [u8; 4] = [0x12, 0xd5, 0x74, 0x98]; // call_prevrandao_then_difficulty()
        let calls = vec![Call {
            selector: call_prevrandao_then_difficulty,
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
    fn cheatcode_difficulty_overwrite_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeDifficulty.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_difficulty: [u8; 4] = [0x59, 0x4a, 0x94, 0x30]; // call_difficulty(uint256)
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let mut args1 = vec![0u8; 32];
        args1[31] = 1;
        let mut args2 = vec![0u8; 32];
        args2[31] = 2;
        let calls = vec![
            Call {
                selector: call_difficulty,
                args: args1,
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_difficulty,
                args: args2,
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
    fn cheatcode_difficulty_zero_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeDifficulty.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_difficulty_zero: [u8; 4] = [0xa5, 0xd5, 0x9c, 0xc5]; // call_difficulty_zero()
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let calls = vec![
            Call {
                selector: call_difficulty_zero,
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
    fn cheatcode_difficulty_max_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeDifficulty.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_difficulty_max: [u8; 4] = [0x8d, 0xe4, 0x75, 0x24]; // call_difficulty_max()
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let calls = vec![
            Call {
                selector: call_difficulty_max,
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
    fn cheatcode_difficulty_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeDifficulty.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_difficulty: [u8; 4] = [0x59, 0x4a, 0x94, 0x30]; // call_difficulty(uint256)
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls_a = vec![Call {
            selector: call_difficulty,
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
    fn cheatcode_difficulty_invariant_final_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeDifficulty.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_noop: [u8; 4] = [0x0a, 0xd4, 0xeb, 0x0c]; // call_noop()
        let calls = vec![Call {
            selector: call_noop,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }
}
