//! Parse family of cheatcodes (`parseUint`, `parseInt`, `parseBool`,
//! `parseAddress`, `parseBytes`, `parseBytes32`).

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::Bytes;

use crate::vm::{Cheatcode, CheatcodeEffect};

fn decode_single(input: &Bytes, t: DynSolType) -> Option<DynSolValue> {
    let tuple = DynSolType::Tuple(vec![t]);
    let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
    match decoded {
        DynSolValue::Tuple(v) => v.into_iter().next(),
        _ => None,
    }
}

/// Parse `s` as `ty` using the same two-layer strategy as Foundry:
///
/// 1. `DynSolType::coerce_str` (strict)
/// 2. `parse_value_fallback` (lenient bool 0/1)
///
/// Returns ABI-encoded bytes or a revert reason.
fn parse(s: &str, ty: &DynSolType) -> Result<Vec<u8>, String> {
    let value = if let Some(v) = parse_value_fallback(s, ty) {
        v
    } else {
        ty.coerce_str(s)
            .map_err(|e| format!("malformed string: {e}"))?
    };
    Ok(value.abi_encode())
}

/// Foundry-compatible fallback for values `coerce_str` does not accept.
fn parse_value_fallback(s: &str, ty: &DynSolType) -> Option<DynSolValue> {
    if *ty == DynSolType::Bool {
        let t = s.trim();
        if t.eq_ignore_ascii_case("true") || t == "1" {
            Some(DynSolValue::Bool(true))
        } else if t.eq_ignore_ascii_case("false") || t == "0" {
            Some(DynSolValue::Bool(false))
        } else {
            None
        }
    } else {
        None
    }
}

pub struct ParseUint;
impl Cheatcode for ParseUint {
    type Args = String;
    const SELECTOR: [u8; 4] = [0xfa, 0x91, 0x45, 0x4d];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        let val = decode_single(input, DynSolType::String)?;
        match val {
            DynSolValue::String(s) => Some(s),
            _ => None,
        }
    }
    fn effects(s: Self::Args) -> Vec<CheatcodeEffect> {
        match parse(&s, &DynSolType::Uint(256)) {
            Ok(bytes) => vec![CheatcodeEffect::ReturnBytes(bytes)],
            Err(_) => vec![CheatcodeEffect::Revert(
                "parseUint: malformed string".into(),
            )],
        }
    }
}

pub struct ParseInt;
impl Cheatcode for ParseInt {
    type Args = String;
    const SELECTOR: [u8; 4] = [0x42, 0x34, 0x6c, 0x5e];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        let val = decode_single(input, DynSolType::String)?;
        match val {
            DynSolValue::String(s) => Some(s),
            _ => None,
        }
    }
    fn effects(s: Self::Args) -> Vec<CheatcodeEffect> {
        match parse(&s, &DynSolType::Int(256)) {
            Ok(bytes) => vec![CheatcodeEffect::ReturnBytes(bytes)],
            Err(_) => vec![CheatcodeEffect::Revert("parseInt: malformed string".into())],
        }
    }
}

pub struct ParseBool;
impl Cheatcode for ParseBool {
    type Args = String;
    const SELECTOR: [u8; 4] = [0x97, 0x4e, 0xf9, 0x24];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        let val = decode_single(input, DynSolType::String)?;
        match val {
            DynSolValue::String(s) => Some(s),
            _ => None,
        }
    }
    fn effects(s: Self::Args) -> Vec<CheatcodeEffect> {
        match parse(&s, &DynSolType::Bool) {
            Ok(bytes) => {
                // ABI-encoded bool is 32 bytes, last byte 0x01 or 0x00.
                let b = bytes.last().copied() == Some(1);
                vec![CheatcodeEffect::ReturnBool(b)]
            }
            Err(_) => vec![CheatcodeEffect::Revert(
                "parseBool: malformed string".into(),
            )],
        }
    }
}

pub struct ParseAddress;
impl Cheatcode for ParseAddress {
    type Args = String;
    const SELECTOR: [u8; 4] = [0xc6, 0xce, 0x05, 0x9d];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        let val = decode_single(input, DynSolType::String)?;
        match val {
            DynSolValue::String(s) => Some(s),
            _ => None,
        }
    }
    fn effects(s: Self::Args) -> Vec<CheatcodeEffect> {
        match parse(&s, &DynSolType::Address) {
            Ok(bytes) => vec![CheatcodeEffect::ReturnBytes(bytes)],
            Err(_) => vec![CheatcodeEffect::Revert(
                "parseAddress: malformed string".into(),
            )],
        }
    }
}

pub struct ParseBytes;
impl Cheatcode for ParseBytes {
    type Args = String;
    const SELECTOR: [u8; 4] = [0x8f, 0x5d, 0x23, 0x2d];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        let val = decode_single(input, DynSolType::String)?;
        match val {
            DynSolValue::String(s) => Some(s),
            _ => None,
        }
    }
    fn effects(s: Self::Args) -> Vec<CheatcodeEffect> {
        match parse(&s, &DynSolType::Bytes) {
            Ok(bytes) => vec![CheatcodeEffect::ReturnBytes(bytes)],
            Err(_) => vec![CheatcodeEffect::Revert(
                "parseBytes: malformed string".into(),
            )],
        }
    }
}

pub struct ParseBytes32;
impl Cheatcode for ParseBytes32 {
    type Args = String;
    const SELECTOR: [u8; 4] = [0x08, 0x7e, 0x6e, 0x81];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        let val = decode_single(input, DynSolType::String)?;
        match val {
            DynSolValue::String(s) => Some(s),
            _ => None,
        }
    }
    fn effects(s: Self::Args) -> Vec<CheatcodeEffect> {
        match parse(&s, &DynSolType::FixedBytes(32)) {
            Ok(bytes) => vec![CheatcodeEffect::ReturnBytes(bytes)],
            Err(_) => vec![CheatcodeEffect::Revert(
                "parseBytes32: malformed string".into(),
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use alloy_primitives::{I256, U256};
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;
    use crate::vm::CheatcodeEffect;

    fn call_data(selector: [u8; 4], encoded: Vec<u8>) -> Bytes {
        let mut data = selector.to_vec();
        data.extend(encoded);
        Bytes::from(data)
    }

    #[test]
    fn parse_uint_decode_and_effects() {
        let encoded = DynSolValue::String("456".into()).abi_encode();
        let args = ParseUint::decode(&call_data(ParseUint::SELECTOR, encoded)).unwrap();
        let effects = ParseUint::effects(args);
        let expected = DynSolValue::Uint(U256::from(456u64), 256).abi_encode();
        assert_eq!(effects, vec![CheatcodeEffect::ReturnBytes(expected)]);
    }

    #[test]
    fn parse_uint_hex() {
        let encoded = DynSolValue::String("0xff".into()).abi_encode();
        let args = ParseUint::decode(&call_data(ParseUint::SELECTOR, encoded)).unwrap();
        let effects = ParseUint::effects(args);
        let expected = DynSolValue::Uint(U256::from(255u64), 256).abi_encode();
        assert_eq!(effects, vec![CheatcodeEffect::ReturnBytes(expected)]);
    }

    #[test]
    fn parse_uint_max() {
        let s = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        let encoded = DynSolValue::String(s.into()).abi_encode();
        let args = ParseUint::decode(&call_data(ParseUint::SELECTOR, encoded)).unwrap();
        let effects = ParseUint::effects(args);
        let expected = DynSolValue::Uint(U256::MAX, 256).abi_encode();
        assert_eq!(effects, vec![CheatcodeEffect::ReturnBytes(expected)]);
    }

    #[test]
    fn parse_uint_reverts_malformed() {
        let encoded = DynSolValue::String("abc".into()).abi_encode();
        let args = ParseUint::decode(&call_data(ParseUint::SELECTOR, encoded)).unwrap();
        let effects = ParseUint::effects(args);
        assert!(matches!(
            effects[0],
            CheatcodeEffect::Revert(ref r) if r.contains("parseUint")
        ));
    }

    #[test]
    fn parse_int_negative() {
        let encoded = DynSolValue::String("-123".into()).abi_encode();
        let args = ParseInt::decode(&call_data(ParseInt::SELECTOR, encoded)).unwrap();
        let effects = ParseInt::effects(args);
        let expected = DynSolValue::Int(I256::try_from(-123i64).unwrap(), 256).abi_encode();
        assert_eq!(effects, vec![CheatcodeEffect::ReturnBytes(expected)]);
    }

    #[test]
    fn parse_int_min() {
        let s = "-57896044618658097711785492504343953926634992332820282019728792003956564819968";
        let encoded = DynSolValue::String(s.into()).abi_encode();
        let args = ParseInt::decode(&call_data(ParseInt::SELECTOR, encoded)).unwrap();
        let effects = ParseInt::effects(args);
        let expected = DynSolValue::Int(I256::MIN, 256).abi_encode();
        assert_eq!(effects, vec![CheatcodeEffect::ReturnBytes(expected)]);
    }

    #[test]
    fn parse_bool_true() {
        for s in ["true", "TRUE", "True", "1"] {
            let encoded = DynSolValue::String(s.into()).abi_encode();
            let args = ParseBool::decode(&call_data(ParseBool::SELECTOR, encoded)).unwrap();
            let effects = ParseBool::effects(args);
            assert_eq!(
                effects,
                vec![CheatcodeEffect::ReturnBool(true)],
                "failed for {}",
                s
            );
        }
    }

    #[test]
    fn parse_bool_false() {
        for s in ["false", "FALSE", "False", "0"] {
            let encoded = DynSolValue::String(s.into()).abi_encode();
            let args = ParseBool::decode(&call_data(ParseBool::SELECTOR, encoded)).unwrap();
            let effects = ParseBool::effects(args);
            assert_eq!(
                effects,
                vec![CheatcodeEffect::ReturnBool(false)],
                "failed for {}",
                s
            );
        }
    }

    #[test]
    fn parse_bool_malformed_reverts() {
        let encoded = DynSolValue::String("maybe".into()).abi_encode();
        let args = ParseBool::decode(&call_data(ParseBool::SELECTOR, encoded)).unwrap();
        let effects = ParseBool::effects(args);
        assert!(matches!(
            effects[0],
            CheatcodeEffect::Revert(ref r) if r.contains("parseBool")
        ));
    }

    #[test]
    fn parse_address_works() {
        let encoded =
            DynSolValue::String("0x71C7656EC7ab88b098defB751B7401B5f6d8976F".into()).abi_encode();
        let args = ParseAddress::decode(&call_data(ParseAddress::SELECTOR, encoded)).unwrap();
        let effects = ParseAddress::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        assert_eq!(
            &out[12..32],
            hex::decode("71c7656ec7ab88b098defb751b7401b5f6d8976f").unwrap()
        );
    }

    #[test]
    fn parse_address_no_prefix() {
        let encoded =
            DynSolValue::String("71C7656EC7ab88b098defB751B7401B5f6d8976F".into()).abi_encode();
        let args = ParseAddress::decode(&call_data(ParseAddress::SELECTOR, encoded)).unwrap();
        let effects = ParseAddress::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        assert_eq!(
            &out[12..32],
            hex::decode("71c7656ec7ab88b098defb751b7401b5f6d8976f").unwrap()
        );
    }

    #[test]
    fn parse_address_reverts_short() {
        let encoded = DynSolValue::String("0x1234".into()).abi_encode();
        let args = ParseAddress::decode(&call_data(ParseAddress::SELECTOR, encoded)).unwrap();
        let effects = ParseAddress::effects(args);
        assert!(matches!(
            effects[0],
            CheatcodeEffect::Revert(ref r) if r.contains("parseAddress")
        ));
    }

    #[test]
    fn parse_bytes_works() {
        let encoded = DynSolValue::String("0xabcd".into()).abi_encode();
        let args = ParseBytes::decode(&call_data(ParseBytes::SELECTOR, encoded)).unwrap();
        let effects = ParseBytes::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let expected = DynSolValue::Bytes(hex::decode("abcd").unwrap()).abi_encode();
        assert_eq!(*out, expected);
    }

    #[test]
    fn parse_bytes_reverts_non_hex() {
        let encoded = DynSolValue::String("hello".into()).abi_encode();
        let args = ParseBytes::decode(&call_data(ParseBytes::SELECTOR, encoded)).unwrap();
        let effects = ParseBytes::effects(args);
        assert!(matches!(
            effects[0],
            CheatcodeEffect::Revert(ref r) if r.contains("parseBytes")
        ));
    }

    #[test]
    fn parse_bytes32_works() {
        let s = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let encoded = DynSolValue::String(s.into()).abi_encode();
        let args = ParseBytes32::decode(&call_data(ParseBytes32::SELECTOR, encoded)).unwrap();
        let effects = ParseBytes32::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        assert_eq!(
            *out,
            hex::decode("1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap()
        );
    }

    #[test]
    fn parse_bytes32_reverts_short() {
        let encoded = DynSolValue::String("0x1234".into()).abi_encode();
        let args = ParseBytes32::decode(&call_data(ParseBytes32::SELECTOR, encoded)).unwrap();
        let effects = ParseBytes32::effects(args);
        assert!(matches!(
            effects[0],
            CheatcodeEffect::Revert(ref r) if r.contains("parseBytes32")
        ));
    }

    #[test]
    #[serial]
    fn cheatcode_parse_setup_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let output = chain.execute(&[]).unwrap();
        assert!(output.all_ok, "setup properties should pass");
    }

    #[test]
    #[serial]
    fn cheatcode_parse_sequence_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_parse_and_store: [u8; 4] = [0x7b, 0x47, 0xb7, 0xab];
        let encoded = DynSolValue::String("42".into()).abi_encode();
        let calls = vec![Call {
            selector: call_parse_and_store,
            args: encoded,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "sequence call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_parse_pure_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_parse_no_side_effect: [u8; 4] = [0xfd, 0xe9, 0x9f, 0x77];
        let encoded = DynSolValue::String("123".into()).abi_encode();
        let calls = vec![Call {
            selector: call_parse_no_side_effect,
            args: encoded,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "pure isolation call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_parse_revert_malformed_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_parse_and_revert: [u8; 4] = [0x55, 0xef, 0x2b, 0x8a];
        let encoded = DynSolValue::String("not a number".into()).abi_encode();
        let calls = vec![Call {
            selector: call_parse_and_revert,
            args: encoded,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(!output.all_ok, "malformed parse should revert");
    }

    #[test]
    #[serial]
    fn cheatcode_parse_cross_deal_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_parse_then_deal: [u8; 4] = [0xdf, 0x49, 0x23, 0x83];
        let encoded = DynSolValue::String("1000".into()).abi_encode();
        let calls = vec![Call {
            selector: call_parse_then_deal,
            args: encoded,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "cross cheatcode call should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_parse_round_trip_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let output = chain.execute(&[]).unwrap();
        assert!(output.all_ok, "round trip properties should pass");
    }

    #[test]
    #[serial]
    fn cheatcode_parse_max_values_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let output = chain.execute(&[]).unwrap();
        assert!(output.all_ok, "max value properties should pass");
    }

    #[test]
    #[serial]
    fn cheatcode_parse_hex_inputs_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let output = chain.execute(&[]).unwrap();
        assert!(output.all_ok, "hex input properties should pass");
    }

    #[test]
    #[serial]
    fn cheatcode_parse_bool_variants_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let output = chain.execute(&[]).unwrap();
        assert!(output.all_ok, "bool variant properties should pass");
    }

    #[test]
    #[serial]
    fn cheatcode_parse_corpus_isolation_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeParse.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let call_parse_and_store: [u8; 4] = [0x7b, 0x47, 0xb7, 0xab];
        let encoded = DynSolValue::String("42".into()).abi_encode();
        let calls_a = vec![Call {
            selector: call_parse_and_store,
            args: encoded,
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];
        let output_a = chain.execute(&calls_a).unwrap();
        assert!(output_a.all_ok, "sequence A should succeed");

        let output_b = chain.execute(&[]).unwrap();
        assert!(output_b.all_ok, "sequence B should succeed");
    }
}
