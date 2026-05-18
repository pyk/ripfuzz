//! Nonce manipulation cheatcodes (`setNonce`, `getNonce`).

use revm::primitives::{Address, Bytes, U256};

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

pub struct SetNonce;

impl Cheatcode for SetNonce {
    type Args = (Address, u64);
    const SELECTOR: [u8; 4] = [0xf8, 0xe1, 0x8b, 0x57];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 + 64 {
            return None;
        }
        let addr = Address::from_slice(&input[4 + 12..4 + 32]);
        let nonce = u64::try_from(U256::from_be_slice(&input[4 + 32..4 + 64])).ok()?;
        Some((addr, nonce))
    }

    fn effects((addr, nonce): Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetAccountNonce(addr, nonce)]
    }
}

pub struct GetNonce;

impl Cheatcode for GetNonce {
    type Args = Address;
    const SELECTOR: [u8; 4] = [0x2d, 0x03, 0x35, 0xab];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 + 32 {
            return None;
        }
        Some(Address::from_slice(&input[4 + 12..4 + 32]))
    }

    fn effects(addr: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::ReadNonce(addr)]
    }
}

/// Snapshot of an address nonce before `setNonce` was applied.
/// Required for rollback on revert (Foundry-compatible semantics).
#[derive(Clone, Debug, PartialEq)]
pub struct NonceRecord {
    pub address: Address,
    pub old_nonce: u64,
    pub new_nonce: u64,
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
    fn set_nonce_decode_and_effects() {
        let mut data = SetNonce::SELECTOR.to_vec();
        let addr = Address::new([0xab; 20]);
        let nonce: u64 = 42;
        let mut padded_addr = vec![0u8; 32];
        padded_addr[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded_addr);
        data.extend_from_slice(&U256::from(nonce).to_be_bytes_vec());
        let args = SetNonce::decode(&Bytes::from(data)).unwrap();
        let effects = SetNonce::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::SetAccountNonce(addr, nonce)]);
    }

    #[test]
    fn get_nonce_decode_and_effects() {
        let mut data = GetNonce::SELECTOR.to_vec();
        let addr = Address::new([0xcd; 20]);
        let mut padded_addr = vec![0u8; 32];
        padded_addr[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded_addr);
        let args = GetNonce::decode(&Bytes::from(data)).unwrap();
        let effects = GetNonce::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::ReadNonce(addr)]);
    }

    #[test]
    fn set_nonce_effect_applies() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xcd; 20]);
        let nonce: u64 = 7;
        let effects = SetNonce::effects((addr, nonce));
        for e in &effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        let info = ctx.journal_mut().load_account(addr).unwrap().data;
        assert_eq!(info.info.nonce, nonce);
    }

    #[test]
    fn get_nonce_read_effect() {
        let _inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xef; 20]);
        let mut info = revm::state::AccountInfo::default();
        info.nonce = 42;
        ctx.db_mut().insert_account_info(addr, info);
        let effects = GetNonce::effects(addr);
        assert_eq!(effects, vec![CheatcodeEffect::ReadNonce(addr)]);
    }

    #[test]
    #[serial]
    fn cheatcode_nonce_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0x4a, 0x77, 0x6c, 0xd7]; // call_record_nonce()
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
    fn cheatcode_nonce_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_set: [u8; 4] = [0x7c, 0x2e, 0x07, 0x7d]; // call_set_nonce(uint64)
        let call_record: [u8; 4] = [0x69, 0x46, 0x76, 0xb0]; // call_record_target_nonce()
        let mut args = vec![0u8; 32];
        args[24..32].copy_from_slice(&100u64.to_be_bytes());
        let calls = vec![
            Call {
                selector: call_set,
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
    fn cheatcode_nonce_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_revert: [u8; 4] = [0xe8, 0xff, 0x5f, 0x65]; // call_set_nonce_and_revert(uint64)
        let mut args = vec![0u8; 32];
        args[24..32].copy_from_slice(&9999u64.to_be_bytes());
        let calls = vec![Call {
            selector: call_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "set_nonce_and_revert should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_nonce_invalid_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_invalid: [u8; 4] = [0xe9, 0xa1, 0x75, 0xa0]; // call_set_nonce_invalid()
        let calls = vec![Call {
            selector: call_invalid,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "call_set_nonce_invalid should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_nonce_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_100: [u8; 4] = [0x62, 0x48, 0xf0, 0x69]; // call_set_nonce_100()
        let call_200: [u8; 4] = [0x43, 0xbd, 0x31, 0xcf]; // call_set_nonce_200()
        let calls = vec![
            Call {
                selector: call_100,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_200,
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
    fn cheatcode_nonce_zero_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_zero: [u8; 4] = [0x04, 0xe5, 0x75, 0x75]; // call_set_nonce_zero()
        let calls = vec![Call {
            selector: call_zero,
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
    fn cheatcode_nonce_max_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_max: [u8; 4] = [0xbc, 0x2e, 0x2c, 0xff]; // call_set_nonce_max()
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
    fn cheatcode_nonce_empty_address_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_empty: [u8; 4] = [0xc9, 0x9e, 0x35, 0x8a]; // call_set_nonce_empty(uint64)
        let mut args = vec![0u8; 32];
        args[24..32].copy_from_slice(&42u64.to_be_bytes());
        let calls = vec![Call {
            selector: call_empty,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_nonce_eoa_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_eoa: [u8; 4] = [0x1a, 0x55, 0x83, 0xc9]; // call_set_nonce_eoa(uint64)
        let mut args = vec![0u8; 32];
        args[24..32].copy_from_slice(&99u64.to_be_bytes());
        let calls = vec![Call {
            selector: call_eoa,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_nonce_invariant_final_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_100: [u8; 4] = [0x62, 0x48, 0xf0, 0x69]; // call_set_nonce_100()
        let calls = vec![Call {
            selector: call_100,
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
    fn cheatcode_nonce_cross_cheatcode_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_cross: [u8; 4] = [0x4c, 0x23, 0x4c, 0xd2]; // call_set_nonce_and_warp_roll()
        let calls = vec![Call {
            selector: call_cross,
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
    fn cheatcode_nonce_self_overwrite_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_self: [u8; 4] = [0xce, 0xe3, 0xfb, 0x40]; // call_self_set_nonce(uint64)
        let mut args = vec![0u8; 32];
        args[24..32].copy_from_slice(&50u64.to_be_bytes());
        let calls = vec![Call {
            selector: call_self,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_nonce_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeNonce.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_100: [u8; 4] = [0x62, 0x48, 0xf0, 0x69]; // call_set_nonce_100()
        let calls_a = vec![Call {
            selector: call_100,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        let call_setup_only: [u8; 4] = [0xcb, 0x67, 0xed, 0x3c]; // setup_only()
        let calls_b = vec![Call {
            selector: call_setup_only,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_b = chain.execute(&calls_b).unwrap();
        assert!(output_b.all_ok, "sequence B should succeed");
    }
}
