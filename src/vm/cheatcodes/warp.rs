//! `warp` cheatcode — set and persist `block.timestamp`.

use revm::primitives::{Bytes, U256};

use crate::vm::{Cheatcode, CheatcodeEffect, decode_u256_arg};

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

    use revm::primitives::{Bytes, U256};
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;
    use crate::vm::test_harness::run_cheatcode;

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
    #[serial]
    fn cheatcode_warp_setup_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0x4c, 0x8b, 0xbb, 0x55]; // call_record_timestamp()
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
    fn cheatcode_warp_sequence_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_warp: [u8; 4] = [0x5d, 0x57, 0xe2, 0x7f]; // call_warp(uint256)
        let call_record: [u8; 4] = [0x4c, 0x8b, 0xbb, 0x55]; // call_record_timestamp()
        let mut args = vec![0u8; 32];
        args[31] = 100; // U256(100)
        let calls = vec![
            Call {
                selector: call_warp,
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
    fn cheatcode_warp_revert_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_warp_revert: [u8; 4] = [0xc0, 0x7d, 0xae, 0xdf]; // call_warp_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![Call {
            selector: call_warp_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "warp_and_revert should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_warp_delay_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_warp_100: [u8; 4] = [0x14, 0x43, 0x9b, 0x94]; // call_warp_100()
        let call_record: [u8; 4] = [0x4c, 0x8b, 0xbb, 0x55]; // call_record_timestamp()
        let calls = vec![
            Call {
                selector: call_warp_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_record,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 5,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "calls should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_warp_overwrite_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_warp_100: [u8; 4] = [0x14, 0x43, 0x9b, 0x94]; // call_warp_100()
        let call_warp_200: [u8; 4] = [0x5c, 0xb1, 0xed, 0xfe]; // call_warp_200()
        let call_record: [u8; 4] = [0x4c, 0x8b, 0xbb, 0x55]; // call_record_timestamp()
        let calls = vec![
            Call {
                selector: call_warp_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_warp_200,
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
    #[serial]
    fn cheatcode_warp_zero_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_warp_zero: [u8; 4] = [0xb3, 0x97, 0xad, 0xb5];
        let call_record: [u8; 4] = [0x4c, 0x8b, 0xbb, 0x55];
        let calls = vec![
            Call {
                selector: call_warp_zero,
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
    fn cheatcode_warp_max_uint64_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_warp_max: [u8; 4] = [0x94, 0x91, 0xec, 0x37];
        let calls = vec![Call {
            selector: call_warp_max,
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
    fn cheatcode_warp_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_warp: [u8; 4] = [0x5d, 0x57, 0xe2, 0x7f];
        let call_record: [u8; 4] = [0x4c, 0x8b, 0xbb, 0x55];
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls_a = vec![Call {
            selector: call_warp,
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
    fn cheatcode_warp_invariant_final_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_warp_100: [u8; 4] = [0x14, 0x43, 0x9b, 0x94];
        let calls = vec![Call {
            selector: call_warp_100,
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
    fn cheatcode_warp_roll_interaction_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeWarp.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_warp_and_roll: [u8; 4] = [0xb0, 0xd9, 0xad, 0x03];
        let calls = vec![Call {
            selector: call_warp_and_roll,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    fn warp_via_test_harness() {
        let mut data = Warp::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(1234567890u64).to_be_bytes_vec());

        let exec_state = crate::vm::ExecutionState::default();
        let caller = revm::primitives::Address::new([0xde; 20]);
        let (result, new_state) = run_cheatcode(caller, Bytes::from(data), exec_state).unwrap();

        assert!(result.is_success(), "cheatcode should succeed");
        assert_eq!(
            new_state.block.timestamp,
            Some(U256::from(1234567890u64)),
            "timestamp should be updated"
        );
    }
}
