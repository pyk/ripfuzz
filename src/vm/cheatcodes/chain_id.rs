//! `chainId` cheatcode — set and persist the EVM chain ID.

use revm::primitives::{Bytes, U256};

use crate::vm::{Cheatcode, CheatcodeEffect, decode_u256_arg};

pub struct ChainId;

impl Cheatcode for ChainId {
    type Args = U256;
    const SELECTOR: [u8; 4] = [0x40, 0x49, 0xdd, 0xd2];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_u256_arg(input)
    }

    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        if value > U256::from(u64::MAX) {
            return vec![CheatcodeEffect::Revert(
                "chain ID must be less than 2^64".to_string(),
            )];
        }
        vec![CheatcodeEffect::SetChainId(value)]
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

    #[test]
    fn chain_id_decode_and_effects() {
        let mut data = ChainId::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(1337u64).to_be_bytes_vec());
        let args = ChainId::decode(&Bytes::from(data)).unwrap();
        let effects = ChainId::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetChainId(U256::from(1337u64))]
        );
    }

    #[test]
    fn chain_id_decode_zero() {
        let mut data = ChainId::SELECTOR.to_vec();
        data.extend_from_slice(&U256::ZERO.to_be_bytes_vec());
        let args = ChainId::decode(&Bytes::from(data)).unwrap();
        let effects = ChainId::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::SetChainId(U256::ZERO)]);
    }

    #[test]
    fn chain_id_decode_max_u64() {
        let mut data = ChainId::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(u64::MAX).to_be_bytes_vec());
        let args = ChainId::decode(&Bytes::from(data)).unwrap();
        let effects = ChainId::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetChainId(U256::from(u64::MAX))]
        );
    }

    #[test]
    fn chain_id_decode_too_large_reverts() {
        let mut data = ChainId::SELECTOR.to_vec();
        data.extend_from_slice(&(U256::from(u64::MAX) + U256::from(1)).to_be_bytes_vec());
        let args = ChainId::decode(&Bytes::from(data)).unwrap();
        let effects = ChainId::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::Revert(
                "chain ID must be less than 2^64".to_string()
            )]
        );
    }

    #[test]
    #[serial]
    fn cheatcode_chain_id_setup_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
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
    fn cheatcode_chain_id_sequence_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_chain_id: [u8; 4] = [0x03, 0x21, 0x0d, 0xc5]; // call_chain_id(uint256)
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&9999u32.to_be_bytes());
        let calls = vec![
            Call {
                selector: call_chain_id,
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
    fn cheatcode_chain_id_revert_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_chain_id_and_revert: [u8; 4] = [0x4c, 0x29, 0x55, 0x08]; // call_chain_id_and_revert(uint256)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&8888u32.to_be_bytes());
        let calls = vec![Call {
            selector: call_chain_id_and_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "chainId_and_revert should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_chain_id_overwrite_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_chain_id_100: [u8; 4] = [0xb8, 0x7c, 0x71, 0xa3]; // call_chain_id_100()
        let call_chain_id_200: [u8; 4] = [0x2e, 0xc7, 0x8f, 0x66]; // call_chain_id_200()
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let calls = vec![
            Call {
                selector: call_chain_id_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_chain_id_200,
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
    fn cheatcode_chain_id_zero_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_chain_id_zero: [u8; 4] = [0xb0, 0xa1, 0xcc, 0xe5]; // call_chain_id_zero()
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let calls = vec![
            Call {
                selector: call_chain_id_zero,
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
    fn cheatcode_chain_id_max_u64_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_chain_id_max: [u8; 4] = [0x7d, 0xf3, 0x12, 0xe9]; // call_chain_id_max_u64()
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let calls = vec![
            Call {
                selector: call_chain_id_max,
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
    fn cheatcode_chain_id_too_large_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_chain_id_too_large: [u8; 4] = [0x2c, 0x93, 0xcd, 0x68]; // call_chain_id_too_large()
        let calls = vec![Call {
            selector: call_chain_id_too_large,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "chainId too large should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_chain_id_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_chain_id: [u8; 4] = [0x03, 0x21, 0x0d, 0xc5]; // call_chain_id(uint256)
        let call_record: [u8; 4] = [0x8f, 0xbd, 0x24, 0x95]; // call_record()
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&7777u32.to_be_bytes());
        let calls_a = vec![Call {
            selector: call_chain_id,
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
    fn cheatcode_chain_id_invariant_final_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_chain_id_100: [u8; 4] = [0xb8, 0x7c, 0x71, 0xa3]; // call_chain_id_100()
        let calls = vec![Call {
            selector: call_chain_id_100,
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
    fn cheatcode_chain_id_warp_interaction_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeChainId.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_chain_id_and_warp: [u8; 4] = [0xc1, 0x4e, 0x5a, 0xe5]; // call_chain_id_and_warp()
        let calls = vec![Call {
            selector: call_chain_id_and_warp,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }
}
