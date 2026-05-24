//! `roll` cheatcode - set and persist `block.number`.

use revm::primitives::{Bytes, U256};

use crate::evm::cheatcode::{Cheatcode, CheatcodeEffect, decode_u256_arg};

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

    use revm::primitives::U256;
    use serial_test::serial;

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
    #[serial]
    fn cheatcode_roll_setup_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0x57, 0xbd, 0x90, 0xb1]; // call_record_block_number()
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
    fn cheatcode_roll_sequence_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_roll: [u8; 4] = [0x37, 0xd4, 0x7f, 0xea]; // call_roll(uint256)
        let call_record: [u8; 4] = [0x57, 0xbd, 0x90, 0xb1]; // call_record_block_number()
        let mut args = vec![0u8; 32];
        args[31] = 100; // U256(100)
        let calls = vec![
            Call {
                selector: call_roll,
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
    fn cheatcode_roll_revert_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_roll_revert: [u8; 4] = [0xa7, 0xf3, 0x89, 0x63]; // call_roll_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![Call {
            selector: call_roll_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "roll_and_revert should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_roll_delay_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_roll_100: [u8; 4] = [0x67, 0xcb, 0xc1, 0x8d]; // call_roll_100()
        let call_record: [u8; 4] = [0x57, 0xbd, 0x90, 0xb1]; // call_record_block_number()
        let calls = vec![
            Call {
                selector: call_roll_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_record,
                args: vec![],
                block_number_delay: 5,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "calls should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_roll_overwrite_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_roll_100: [u8; 4] = [0x67, 0xcb, 0xc1, 0x8d]; // call_roll_100()
        let call_roll_200: [u8; 4] = [0xf8, 0x5a, 0x7f, 0x34]; // call_roll_200()
        let call_record: [u8; 4] = [0x57, 0xbd, 0x90, 0xb1]; // call_record_block_number()
        let calls = vec![
            Call {
                selector: call_roll_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_roll_200,
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
    fn cheatcode_roll_zero_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_roll_zero: [u8; 4] = [0x1a, 0xf3, 0xcf, 0x35]; // call_roll_zero()
        let call_record: [u8; 4] = [0x57, 0xbd, 0x90, 0xb1]; // call_record_block_number()
        let calls = vec![
            Call {
                selector: call_roll_zero,
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
    fn cheatcode_roll_max_uint64_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_roll_max: [u8; 4] = [0x2d, 0x20, 0xd5, 0xfa]; // call_roll_max_uint64()
        let calls = vec![Call {
            selector: call_roll_max,
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
    fn cheatcode_roll_corpus_isolation_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_roll: [u8; 4] = [0x37, 0xd4, 0x7f, 0xea]; // call_roll(uint256)
        let call_record: [u8; 4] = [0x57, 0xbd, 0x90, 0xb1]; // call_record_block_number()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls_a = vec![Call {
            selector: call_roll,
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
    fn cheatcode_roll_invariant_final_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_roll_100: [u8; 4] = [0x67, 0xcb, 0xc1, 0x8d]; // call_roll_100()
        let calls = vec![Call {
            selector: call_roll_100,
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
    fn cheatcode_roll_warp_interaction_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeRoll.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_roll_and_warp: [u8; 4] = [0x1e, 0x0d, 0x2f, 0xf8]; // call_roll_and_warp()
        let calls = vec![Call {
            selector: call_roll_and_warp,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }
}
