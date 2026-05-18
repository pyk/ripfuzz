//! Storage read / write cheatcodes (`vm.load`, `vm.store`).

use revm::primitives::{Address, Bytes, U256};

use crate::chain::cheatcodes::{
    Cheatcode, CheatcodeEffect, decode_address_bytes32_args, decode_address_bytes32_bytes32_args,
};

pub struct Load;

impl Cheatcode for Load {
    type Args = (Address, [u8; 32]);
    const SELECTOR: [u8; 4] = [0x66, 0x7f, 0x9d, 0x70];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_bytes32_args(input)
    }

    fn effects((addr, slot): Self::Args) -> Vec<CheatcodeEffect> {
        let slot_u256 = U256::from_be_bytes(slot);
        vec![CheatcodeEffect::ReadStorage(addr, slot_u256)]
    }
}

pub struct Store;

impl Cheatcode for Store {
    type Args = (Address, [u8; 32], [u8; 32]);
    const SELECTOR: [u8; 4] = [0x70, 0xca, 0x10, 0xbb];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_bytes32_bytes32_args(input)
    }

    fn effects((addr, slot, value): Self::Args) -> Vec<CheatcodeEffect> {
        let slot_u256 = U256::from_be_bytes(slot);
        let value_u256 = U256::from_be_bytes(value);
        vec![CheatcodeEffect::SetStorage(addr, slot_u256, value_u256)]
    }
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
    use crate::chain::cheatcodes::build_outcome;
    use crate::chain::cheatcodes::effect::apply_effect;
    use crate::chain::inspectors::cheatcode::CheatcodeInspector;
    use crate::contract;
    use crate::corpus::Call;

    // ------------------------------------------------------------------
    //  Load unit tests
    // ------------------------------------------------------------------

    #[test]
    fn load_decode_and_effects() {
        let mut data = Load::SELECTOR.to_vec();
        let addr = Address::new([0xab; 20]);
        let slot = [0xcdu8; 32];
        let mut padded_addr = vec![0u8; 32];
        padded_addr[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded_addr);
        data.extend_from_slice(&slot);
        let args = Load::decode(&Bytes::from(data)).unwrap();
        let effects = Load::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::ReadStorage(
                addr,
                U256::from_be_bytes(slot)
            )]
        );
    }

    #[test]
    fn load_effect_reads_storage() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0x11; 20]);
        let slot = [0x22u8; 32];
        let value = [0x33u8; 32];

        // Store a value.
        let store_effects = Store::effects((addr, slot, value));
        for e in &store_effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }

        // Read it back via build_outcome.
        let load_effects = Load::effects((addr, slot));
        let outcome = build_outcome(&load_effects, 1_000_000, &mut ctx, &inspector.state);
        assert_eq!(
            outcome.result.result,
            revm::interpreter::InstructionResult::Return
        );
        let expected = U256::from_be_bytes(value).to_be_bytes_vec();
        assert_eq!(outcome.result.output, Bytes::from(expected));
    }

    // ------------------------------------------------------------------
    //  Store unit tests
    // ------------------------------------------------------------------

    #[test]
    fn store_decode_and_effects() {
        let mut data = Store::SELECTOR.to_vec();
        let addr = Address::new([0xab; 20]);
        let slot = [0xcdu8; 32];
        let value = [0xefu8; 32];
        let mut padded_addr = vec![0u8; 32];
        padded_addr[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded_addr);
        data.extend_from_slice(&slot);
        data.extend_from_slice(&value);
        let args = Store::decode(&Bytes::from(data)).unwrap();
        let effects = Store::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetStorage(
                addr,
                U256::from_be_bytes(slot),
                U256::from_be_bytes(value)
            )]
        );
    }

    #[test]
    fn store_effect_applies() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0x11; 20]);
        let slot = [0x22u8; 32];
        let value = [0x33u8; 32];
        let store_effects = Store::effects((addr, slot, value));
        for e in &store_effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        let loaded = ctx
            .journal_mut()
            .sload(addr, U256::from_be_bytes(slot))
            .unwrap()
            .data;
        assert_eq!(loaded, U256::from_be_bytes(value));
    }

    // ------------------------------------------------------------------
    //  Load integration tests (CheatcodeLoad.sol)
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn cheatcode_load_setup_persists_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLoad.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0xa1, 0xda, 0x7e, 0x1c]; // call_record_slot_a()
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
    fn cheatcode_load_same_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLoad.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_then_load: [u8; 4] = [0x47, 0x16, 0x49, 0xa5]; // call_store_then_load(bytes32)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xFACADEu32.to_be_bytes());
        let calls = vec![Call {
            selector: call_store_then_load,
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
    fn cheatcode_load_revert_safety_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLoad.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_and_revert: [u8; 4] = [0x9b, 0xc0, 0x03, 0xa3]; // call_store_and_revert(bytes32)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xBADu32.to_be_bytes());
        let calls = vec![Call {
            selector: call_store_and_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "store_and_revert should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_load_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLoad.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_overwrite: [u8; 4] = [0x73, 0x6b, 0xd4, 0x90]; // call_store_overwrite()
        let calls = vec![Call {
            selector: call_store_overwrite,
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
    fn cheatcode_load_empty_address_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLoad.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_load_empty: [u8; 4] = [0xd2, 0x98, 0x9e, 0x0e]; // call_load_empty()
        let calls = vec![Call {
            selector: call_load_empty,
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
    fn cheatcode_load_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLoad.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_then_load: [u8; 4] = [0x47, 0x16, 0x49, 0xa5]; // call_store_then_load(bytes32)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xFACADEu32.to_be_bytes());
        let calls_a = vec![Call {
            selector: call_store_then_load,
            args,
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

    #[test]
    #[serial]
    fn cheatcode_load_cross_cheatcode_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLoad.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_load_and_warp: [u8; 4] = [0xce, 0x3d, 0x66, 0xe5]; // call_load_and_warp()
        let calls = vec![Call {
            selector: call_load_and_warp,
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
    fn cheatcode_load_precompile_reverts_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLoad.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_load_precompile: [u8; 4] = [0x1c, 0x18, 0x63, 0x68]; // call_load_precompile()
        let calls = vec![Call {
            selector: call_load_precompile,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "load on precompile should revert the call");
    }

    // ------------------------------------------------------------------
    //  Store integration tests (CheatcodeStore.sol)
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn store_setup_persists_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_record: [u8; 4] = [0xa1, 0xda, 0x7e, 0x1c]; // call_record_slot_a()
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
    fn store_same_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_then_load: [u8; 4] = [0x47, 0x16, 0x49, 0xa5]; // call_store_then_load(bytes32)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xFACADEu32.to_be_bytes());
        let calls = vec![Call {
            selector: call_store_then_load,
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
    fn store_revert_safety_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_and_revert: [u8; 4] = [0x9b, 0xc0, 0x03, 0xa3]; // call_store_and_revert(bytes32)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xBADu32.to_be_bytes());
        let calls = vec![Call {
            selector: call_store_and_revert,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "store_and_revert should revert");
    }

    #[test]
    #[serial]
    fn store_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_overwrite: [u8; 4] = [0x73, 0x6b, 0xd4, 0x90]; // call_store_overwrite()
        let calls = vec![Call {
            selector: call_store_overwrite,
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
    fn store_zero_clear_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_zero: [u8; 4] = [0xd2, 0x52, 0xd1, 0xbf]; // call_store_zero()
        let calls = vec![Call {
            selector: call_store_zero,
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
    fn store_empty_address_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_empty: [u8; 4] = [0x95, 0x21, 0x71, 0xa8]; // call_store_empty()
        let calls = vec![Call {
            selector: call_store_empty,
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
    fn store_multi_call_final_state_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_step1: [u8; 4] = [0x01, 0x6f, 0x31, 0x93]; // call_store_step1()
        let call_step2: [u8; 4] = [0x17, 0x1c, 0x7e, 0xcf]; // call_store_step2()
        let calls = vec![
            Call {
                selector: call_step1,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_step2,
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
    fn store_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_then_load: [u8; 4] = [0x47, 0x16, 0x49, 0xa5]; // call_store_then_load(bytes32)
        let mut args = vec![0u8; 32];
        args[28..32].copy_from_slice(&0xFACADEu32.to_be_bytes());
        let calls_a = vec![Call {
            selector: call_store_then_load,
            args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        let call_setup_only: [u8; 4] = [0x0e, 0xa6, 0x3b, 0xe6]; // setup_only_store()
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

    #[test]
    #[serial]
    fn store_precompile_reverts_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_precompile: [u8; 4] = [0x77, 0x26, 0x45, 0xc9]; // call_store_precompile()
        let calls = vec![Call {
            selector: call_store_precompile,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "store on precompile should revert the call");
    }

    #[test]
    #[serial]
    fn store_cross_cheatcode_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeStore.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_store_and_warp: [u8; 4] = [0x53, 0xd5, 0x28, 0x9f]; // call_store_and_warp()
        let calls = vec![Call {
            selector: call_store_and_warp,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call should succeed");
    }
}
