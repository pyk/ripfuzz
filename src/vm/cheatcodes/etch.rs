//! Account code manipulation cheatcode (`vm.etch`).

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::{Address, Bytes};

use crate::vm::{Cheatcode, CheatcodeEffect};

pub struct Etch;

impl Cheatcode for Etch {
    type Args = (Address, Bytes);
    const SELECTOR: [u8; 4] = [0xb4, 0xd6, 0xc7, 0x82];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        let tuple = DynSolType::Tuple(vec![DynSolType::Address, DynSolType::Bytes]);
        let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
        let values = match decoded {
            DynSolValue::Tuple(v) => v,
            _ => return None,
        };
        if values.len() != 2 {
            return None;
        }
        let addr = match &values[0] {
            DynSolValue::Address(a) => *a,
            _ => return None,
        };
        let code = match &values[1] {
            DynSolValue::Bytes(b) => Bytes::from(b.clone()),
            _ => return None,
        };
        Some((addr, code))
    }

    fn effects((addr, code): Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetAccountCode(addr, code)]
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
        primitives::Address,
    };
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;
    use crate::vm::effect::apply_effect;
    use crate::vm::inspector::CheatcodeInspector;

    #[test]
    fn etch_effect_applies() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xbe; 20]);
        let code = vec![
            0x60, 0x01, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0x60, 0x00, 0xf3,
        ];

        // Manual ABI encoding for etch(address,bytes):
        let mut data = Etch::SELECTOR.to_vec();
        let mut padded_addr = vec![0u8; 32];
        padded_addr[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded_addr);
        let bytes_offset: u32 = 64;
        let mut offset_word = vec![0u8; 32];
        offset_word[28..32].copy_from_slice(&bytes_offset.to_be_bytes());
        data.extend_from_slice(&offset_word);
        let mut len_word = vec![0u8; 32];
        len_word[28..32].copy_from_slice(&(code.len() as u32).to_be_bytes());
        data.extend_from_slice(&len_word);
        let mut code_padded = code.clone();
        while code_padded.len() % 32 != 0 {
            code_padded.push(0);
        }
        data.extend_from_slice(&code_padded);

        let args = Etch::decode(&Bytes::from(data)).unwrap();
        let effects = Etch::effects(args);
        for e in &effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        let info = ctx.journal_mut().load_account(addr).unwrap().data;
        assert!(info.info.code.is_some());
        assert!(!info.info.code_hash.is_zero());
    }

    // --- Integration tests ---

    #[test]
    #[serial]
    fn cheatcode_etch_setup_persists() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeEtch.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_selector: [u8; 4] = [0x0c, 0xed, 0x93, 0xdf]; // call_record_extcodesize_cafe()
        let calls = vec![Call {
            selector: call_selector,
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
    fn cheatcode_etch_same_sequence_persists() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeEtch.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_etch: [u8; 4] = [0xf4, 0x0e, 0x08, 0x6d]; // call_etch_beef()
        let calls = vec![
            Call {
                selector: call_etch,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_etch,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "sequence should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_etch_corpus_isolation() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeEtch.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_etch: [u8; 4] = [0xf4, 0x0e, 0x08, 0x6d]; // call_etch_beef()
        let call_record: [u8; 4] = [0x0c, 0xed, 0x93, 0xdf]; // call_record_extcodesize_cafe()

        // Sequence A: etch BEEF
        let calls_a = vec![Call {
            selector: call_etch,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];
        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        // Sequence B: should NOT see BEEF etch, but should still see CAFE from setUp
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
    fn cheatcode_etch_revert_undoes() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeEtch.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_revert: [u8; 4] = [0x30, 0x77, 0x83, 0x7b]; // call_etch_and_revert()
        let calls = vec![Call {
            selector: call_revert,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "reverting call should fail sequence");
        // Properties are still checked against the final state.
    }

    #[test]
    #[serial]
    fn cheatcode_etch_overwrite() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeEtch.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_overwrite: [u8; 4] = [0xf0, 0x40, 0x7f, 0x90]; // call_etch_overwrite()
        let calls = vec![Call {
            selector: call_overwrite,
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
    fn cheatcode_etch_new_account() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeEtch.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_new: [u8; 4] = [0x05, 0xa6, 0x5c, 0x5a]; // call_etch_new_account()
        let calls = vec![Call {
            selector: call_new,
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
    fn cheatcode_etch_empty_code_clears() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xcd; 20]);
        let code = Bytes::from(vec![0x60, 0x01, 0x00]);

        // First etch some code
        let effects = Etch::effects((addr, code));
        apply_effect(&effects[0], &mut ctx, &mut inspector.state).unwrap();

        // Then etch empty code
        let empty = Bytes::new();
        let effects_empty = Etch::effects((addr, empty));
        apply_effect(&effects_empty[0], &mut ctx, &mut inspector.state).unwrap();

        let info = ctx.journal_mut().load_account(addr).unwrap().data;
        assert!(
            info.info
                .code
                .as_ref()
                .map(|c: &revm::bytecode::Bytecode| c.is_empty())
                .unwrap_or(true),
            "empty etch should clear code"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_etch_precompile_reverts() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeEtch.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_precompile: [u8; 4] = [0xbb, 0xf4, 0x3e, 0x73]; // call_etch_precompile()
        let calls = vec![Call {
            selector: call_precompile,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(
            !output.all_ok,
            "etching a precompile should revert the call"
        );
    }
}
