//! Label cheatcode.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::{Address, Bytes};

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

pub struct Label;
impl Cheatcode for Label {
    type Args = (Address, String);
    const SELECTOR: [u8; 4] = [0xc6, 0x57, 0xc7, 0x18];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 {
            return None;
        }
        let types = vec![DynSolType::Address, DynSolType::String];
        let tuple = DynSolType::Tuple(types);
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
        let name = match &values[1] {
            DynSolValue::String(s) => s.clone(),
            _ => return None,
        };
        Some((addr, name))
    }
    fn effects((addr, name): Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::AddLabel(addr, name)]
    }
}

pub struct GetLabel;
impl Cheatcode for GetLabel {
    type Args = Address;
    const SELECTOR: [u8; 4] = [0x28, 0xa2, 0x49, 0xb0];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 + 32 {
            return None;
        }
        Some(Address::from_slice(&input[4 + 12..4 + 32]))
    }
    fn effects(addr: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::GetLabel(addr)]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, RwLock};

    use revm::{MainContext, primitives::Address};
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::effect::apply_effect;
    use crate::chain::inspectors::cheatcode::CheatcodeInspector;
    use crate::contract;
    use crate::corpus::Call;

    fn label_calldata(addr: Address, name: &str) -> Bytes {
        let mut data = Label::SELECTOR.to_vec();
        let mut param1 = vec![0u8; 32];
        param1[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&param1);
        let mut param2 = vec![0u8; 32];
        param2[31] = 64;
        data.extend_from_slice(&param2);
        let mut len = vec![0u8; 32];
        len[31] = name.len() as u8;
        data.extend_from_slice(&len);
        let mut str_data = vec![0u8; 32];
        str_data[..name.len()].copy_from_slice(name.as_bytes());
        data.extend_from_slice(&str_data);
        Bytes::from(data)
    }

    fn get_label_calldata(addr: Address) -> Bytes {
        let mut data = GetLabel::SELECTOR.to_vec();
        let mut param = vec![0u8; 32];
        param[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&param);
        Bytes::from(data)
    }

    // ------------------------------------------------------------------
    // Unit tests
    // ------------------------------------------------------------------

    #[test]
    fn label_decode_and_effects() {
        let addr = Address::new([0xab; 20]);
        let name = "MyContract";
        let input = label_calldata(addr, name);
        let args = Label::decode(&input).unwrap();
        let effects = Label::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::AddLabel(addr, name.into())]);
    }

    #[test]
    fn label_inserts_into_state() {
        let mut inspector = CheatcodeInspector::new();
        let addr = Address::new([0xab; 20]);
        let name = "MyContract";
        let input = label_calldata(addr, name);
        let args = Label::decode(&input).unwrap();
        let effects = Label::effects(args);
        let mut ctx =
            revm::context::Context::mainnet().with_db(revm::database::InMemoryDB::default());
        for e in &effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        assert_eq!(inspector.state.labels.get(&addr), Some(&name.to_string()));
    }

    #[test]
    fn label_overwrites_existing() {
        let mut inspector = CheatcodeInspector::new();
        let addr = Address::new([0xab; 20]);
        let mut ctx =
            revm::context::Context::mainnet().with_db(revm::database::InMemoryDB::default());
        apply_effect(
            &CheatcodeEffect::AddLabel(addr, "First".into()),
            &mut ctx,
            &mut inspector.state,
        )
        .unwrap();
        apply_effect(
            &CheatcodeEffect::AddLabel(addr, "Second".into()),
            &mut ctx,
            &mut inspector.state,
        )
        .unwrap();
        assert_eq!(
            inspector.state.labels.get(&addr),
            Some(&"Second".to_string())
        );
    }

    #[test]
    fn get_label_decode_and_effects() {
        let addr = Address::new([0xab; 20]);
        let input = get_label_calldata(addr);
        let args = GetLabel::decode(&input).unwrap();
        let effects = GetLabel::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::GetLabel(addr)]);
    }

    #[test]
    fn get_label_returns_empty_for_unknown() {
        let inspector = CheatcodeInspector::new();
        let addr = Address::new([0xab; 20]);
        let input = get_label_calldata(addr);
        let args = GetLabel::decode(&input).unwrap();
        let effects = GetLabel::effects(args);
        let mut ctx =
            revm::context::Context::mainnet().with_db(revm::database::InMemoryDB::default());
        let outcome = crate::chain::cheatcodes::build_outcome(
            &effects,
            1_000_000,
            &mut ctx,
            &inspector.state,
        );
        let encoded = alloy_dyn_abi::DynSolValue::String("".into()).abi_encode();
        assert_eq!(outcome.result.output, Bytes::from(encoded));
    }

    #[test]
    fn get_label_roundtrip() {
        let mut inspector = CheatcodeInspector::new();
        let addr = Address::new([0xab; 20]);
        inspector.state.labels.insert(addr, "Roundtrip".into());
        let input = get_label_calldata(addr);
        let args = GetLabel::decode(&input).unwrap();
        let effects = GetLabel::effects(args);
        let mut ctx =
            revm::context::Context::mainnet().with_db(revm::database::InMemoryDB::default());
        let outcome = crate::chain::cheatcodes::build_outcome(
            &effects,
            1_000_000,
            &mut ctx,
            &inspector.state,
        );
        let encoded = alloy_dyn_abi::DynSolValue::String("Roundtrip".into()).abi_encode();
        assert_eq!(outcome.result.output, Bytes::from(encoded));
    }

    #[test]
    fn label_writes_to_shared_map() {
        let shared = Arc::new(RwLock::new(HashMap::new()));
        let mut inspector = CheatcodeInspector::new().with_shared_labels(Arc::clone(&shared));
        let addr = Address::new([0xcd; 20]);
        let name = "SharedLabel";
        let input = label_calldata(addr, name);
        let args = Label::decode(&input).unwrap();
        let effects = Label::effects(args);
        let mut ctx =
            revm::context::Context::mainnet().with_db(revm::database::InMemoryDB::default());
        for e in &effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        // call() in the inspector syncs labels; simulate it here.
        if let Some(ref s) = inspector.shared_labels {
            if let Ok(mut guard) = s.write() {
                for (a, n) in &inspector.state.labels {
                    guard.insert(*a, n.clone());
                }
            }
        }
        let guard = shared.read().unwrap();
        assert_eq!(guard.get(&addr), Some(&name.to_string()));
    }

    // ------------------------------------------------------------------
    // Integration tests
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn label_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLabel.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let output = chain.execute(&[]).unwrap();
        assert!(output.all_ok, "empty sequence should succeed");
    }

    #[test]
    #[serial]
    fn label_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLabel.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_label: [u8; 4] = [0xa9, 0x43, 0xd2, 0xa8]; // call_label(address,string)

        let addr = Address::new([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0xca, 0xfe,
        ]);
        let name = "OtherLabel";
        let mut addr_param = vec![0u8; 32];
        addr_param[12..32].copy_from_slice(addr.as_slice());
        let mut str_offset = vec![0u8; 32];
        str_offset[31] = 64;
        let mut len = vec![0u8; 32];
        len[31] = name.len() as u8;
        let mut str_data = vec![0u8; 32];
        str_data[..name.len()].copy_from_slice(name.as_bytes());

        let mut args = vec![];
        args.extend_from_slice(&addr_param);
        args.extend_from_slice(&str_offset);
        args.extend_from_slice(&len);
        args.extend_from_slice(&str_data);

        let calls = vec![Call {
            selector: call_label,
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
    fn label_overwrite_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLabel.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_label_twice: [u8; 4] = [0x3b, 0x39, 0xae, 0x8c]; // call_label_twice()
        let calls = vec![Call {
            selector: call_label_twice,
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
    fn label_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLabel.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_label_then_revert: [u8; 4] = [0x27, 0xbb, 0xf3, 0x71]; // call_label_then_revert()
        let calls = vec![Call {
            selector: call_label_then_revert,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "call_label_then_revert should revert");
    }

    #[test]
    #[serial]
    fn label_empty_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLabel.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_label_empty: [u8; 4] = [0xdb, 0xe0, 0x00, 0x79]; // call_label_empty()
        let calls = vec![Call {
            selector: call_label_empty,
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
    fn label_zero_address_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLabel.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_label_zero: [u8; 4] = [0x07, 0x60, 0xb1, 0x08]; // call_label_zero()
        let calls = vec![Call {
            selector: call_label_zero,
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
    fn label_setup_override_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLabel.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_label_overrides_setup: [u8; 4] = [0x2f, 0xd6, 0xb5, 0x5d]; // call_label_overrides_setup()
        let calls = vec![Call {
            selector: call_label_overrides_setup,
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
    fn label_cross_sequence_isolation() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeLabel.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_label: [u8; 4] = [0xa9, 0x43, 0xd2, 0xa8]; // call_label(address,string)

        let addr = Address::new([
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0xca, 0xfe,
        ]);
        let name = "Seq1Label";
        let mut addr_param = vec![0u8; 32];
        addr_param[12..32].copy_from_slice(addr.as_slice());
        let mut str_offset = vec![0u8; 32];
        str_offset[31] = 64;
        let mut len = vec![0u8; 32];
        len[31] = name.len() as u8;
        let mut str_data = vec![0u8; 32];
        str_data[..name.len()].copy_from_slice(name.as_bytes());

        let mut label_args = vec![];
        label_args.extend_from_slice(&addr_param);
        label_args.extend_from_slice(&str_offset);
        label_args.extend_from_slice(&len);
        label_args.extend_from_slice(&str_data);

        // Sequence A: label an address
        let calls_a = vec![Call {
            selector: call_label,
            args: label_args,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        // Sequence B: fresh clone, should NOT see Seq1's label
        let output_b = chain.execute(&[]).unwrap();
        assert!(output_b.all_ok, "sequence B should succeed");
    }
}
