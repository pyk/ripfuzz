//! `coinbase` cheatcode — set and persist `block.coinbase`.

use revm::primitives::{Address, Bytes};

use crate::vm::{Cheatcode, CheatcodeEffect, decode_address_arg};

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
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodeCoinbase.sol",
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0xe8, 0xc1, 0x48, 0xfb]; // call_record_coinbase()
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
    fn cheatcode_coinbase_sequence_integration() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodeCoinbase.sol",
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_coinbase: [u8; 4] = [0xa3, 0xd2, 0x69, 0x68]; // call_coinbase(address)
        let call_record: [u8; 4] = [0xe8, 0xc1, 0x48, 0xfb]; // call_record_coinbase()
        let mut args = vec![0u8; 32];
        args[31] = 0xAB;
        let calls = vec![
            Call {
                selector: call_coinbase,
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
    fn cheatcode_coinbase_revert_integration() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodeCoinbase.sol",
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_revert: [u8; 4] = [0xd0, 0xec, 0x30, 0xf3]; // call_coinbase_and_revert(address)
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
        assert!(!output.all_ok, "coinbase_and_revert should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_coinbase_overwrite_integration() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodeCoinbase.sol",
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_a: [u8; 4] = [0x7a, 0xae, 0x30, 0x59]; // call_coinbase_A()
        let call_b: [u8; 4] = [0x4a, 0x35, 0x3e, 0x4e]; // call_coinbase_B()
        let call_record: [u8; 4] = [0xe8, 0xc1, 0x48, 0xfb]; // call_record_coinbase()
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
    fn cheatcode_coinbase_zero_integration() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodeCoinbase.sol",
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_zero: [u8; 4] = [0x3b, 0x34, 0x40, 0x8c]; // call_coinbase_zero()
        let call_record: [u8; 4] = [0xe8, 0xc1, 0x48, 0xfb]; // call_record_coinbase()
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
    fn cheatcode_coinbase_corpus_isolation_integration() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodeCoinbase.sol",
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_coinbase: [u8; 4] = [0xa3, 0xd2, 0x69, 0x68]; // call_coinbase(address)
        let call_record: [u8; 4] = [0xe8, 0xc1, 0x48, 0xfb]; // call_record_coinbase()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xDEADu32.to_be_bytes());
        let calls_a = vec![Call {
            selector: call_coinbase,
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
    fn cheatcode_coinbase_invariant_final_integration() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodeCoinbase.sol",
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_a: [u8; 4] = [0x7a, 0xae, 0x30, 0x59]; // call_coinbase_A()
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
    fn cheatcode_coinbase_roll_warp_fee_interaction_integration() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodeCoinbase.sol",
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_interaction: [u8; 4] = [0x55, 0x0b, 0x0c, 0xbe]; // call_coinbase_and_roll_warp_fee()
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
