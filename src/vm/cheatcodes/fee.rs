//! `fee` cheatcode — set and persist `block.basefee`.

use revm::primitives::{Bytes, U256};

use crate::vm::{Cheatcode, CheatcodeEffect, decode_u256_arg};

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

    use revm::primitives::U256;
    use serial_test::serial;

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
    #[serial]
    fn cheatcode_fee_setup_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFee.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0xbc, 0xfa, 0x34, 0x3e]; // call_record_basefee()
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
    fn cheatcode_fee_sequence_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFee.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_fee: [u8; 4] = [0xa0, 0x67, 0x5b, 0x95]; // call_fee(uint256)
        let call_record: [u8; 4] = [0xbc, 0xfa, 0x34, 0x3e]; // call_record_basefee()
        let mut args = vec![0u8; 32];
        args[31] = 100; // U256(100)
        let calls = vec![
            Call {
                selector: call_fee,
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
    fn cheatcode_fee_revert_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFee.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_fee_revert: [u8; 4] = [0x22, 0xfa, 0x48, 0x0c]; // call_fee_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![Call {
            selector: call_fee_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "fee_and_revert should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_fee_overwrite_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFee.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_fee_100: [u8; 4] = [0xf8, 0xf9, 0x27, 0xd6]; // call_fee_100()
        let call_fee_200: [u8; 4] = [0x5d, 0x41, 0xb8, 0xfb]; // call_fee_200()
        let call_record: [u8; 4] = [0xbc, 0xfa, 0x34, 0x3e]; // call_record_basefee()
        let calls = vec![
            Call {
                selector: call_fee_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_fee_200,
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
    fn cheatcode_fee_zero_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFee.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_fee_zero: [u8; 4] = [0x99, 0xe5, 0x90, 0x06]; // call_fee_zero()
        let call_record: [u8; 4] = [0xbc, 0xfa, 0x34, 0x3e]; // call_record_basefee()
        let calls = vec![
            Call {
                selector: call_fee_zero,
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
    fn cheatcode_fee_max_uint64_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFee.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_fee_max: [u8; 4] = [0x5b, 0xf7, 0xaa, 0x07]; // call_fee_max_uint64()
        let calls = vec![Call {
            selector: call_fee_max,
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
    fn cheatcode_fee_corpus_isolation_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFee.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_fee: [u8; 4] = [0xa0, 0x67, 0x5b, 0x95]; // call_fee(uint256)
        let call_record: [u8; 4] = [0xbc, 0xfa, 0x34, 0x3e]; // call_record_basefee()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls_a = vec![Call {
            selector: call_fee,
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
    fn cheatcode_fee_invariant_final_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFee.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_fee_100: [u8; 4] = [0xf8, 0xf9, 0x27, 0xd6]; // call_fee_100()
        let calls = vec![Call {
            selector: call_fee_100,
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
    fn cheatcode_fee_roll_warp_interaction_integration() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodeFee.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_fee_and_roll_warp: [u8; 4] = [0x88, 0x02, 0x2d, 0xe1]; // call_fee_and_roll_warp()
        let calls = vec![Call {
            selector: call_fee_and_roll_warp,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }
}
