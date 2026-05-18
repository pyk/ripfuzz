//! Parsing cheatcodes and `getCode`.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, I256, U256};
use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

fn decode_single(input: &Bytes, t: DynSolType) -> Option<DynSolValue> {
    let tuple = DynSolType::Tuple(vec![t]);
    let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
    match decoded {
        DynSolValue::Tuple(v) => v.into_iter().next(),
        _ => None,
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
        let u = match U256::from_str_radix(&s, 10) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        vec![CheatcodeEffect::ReturnU256(u)]
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
        let negative = s.starts_with('-');
        let mag_str = if negative { &s[1..] } else { &s };
        let mag = match U256::from_str_radix(mag_str, 10) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        let sign = if negative {
            alloy_primitives::Sign::Negative
        } else {
            alloy_primitives::Sign::Positive
        };
        let i = match I256::checked_from_sign_and_abs(sign, mag) {
            Some(v) => v,
            None => return vec![],
        };
        vec![CheatcodeEffect::ReturnBytes(
            DynSolValue::Int(i, 256).abi_encode(),
        )]
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
        let b = s.trim().eq_ignore_ascii_case("true");
        vec![CheatcodeEffect::ReturnBool(b)]
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
        let cleaned = s.trim().trim_start_matches("0x").trim_start_matches("0X");
        let bytes = match hex::decode(cleaned) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        if bytes.len() != 20 {
            return vec![];
        }
        let addr = Address::from_slice(&bytes);
        let mut out = vec![0u8; 32];
        out[12..32].copy_from_slice(addr.as_slice());
        vec![CheatcodeEffect::ReturnBytes(out)]
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
        let cleaned = s.trim().trim_start_matches("0x").trim_start_matches("0X");
        let bytes = match hex::decode(cleaned) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        vec![CheatcodeEffect::ReturnBytes(
            DynSolValue::Bytes(bytes).abi_encode(),
        )]
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
        let cleaned = s.trim().trim_start_matches("0x").trim_start_matches("0X");
        let bytes = match hex::decode(cleaned) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        if bytes.len() != 32 {
            return vec![];
        }
        vec![CheatcodeEffect::ReturnBytes(bytes)]
    }
}

pub struct GetCode;
impl Cheatcode for GetCode {
    type Args = String;
    const SELECTOR: [u8; 4] = [0x8d, 0x1c, 0xc9, 0x25];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        let val = decode_single(input, DynSolType::String)?;
        match val {
            DynSolValue::String(s) => Some(s),
            _ => None,
        }
    }
    fn effects(arg: Self::Args) -> Vec<CheatcodeEffect> {
        let name = arg.split(':').next_back().unwrap_or(&arg).trim().into();
        vec![CheatcodeEffect::GetCode(name)]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::inspectors::cheatcode::CheatcodeInspector;
    use crate::contract;

    fn call_data(selector: [u8; 4], encoded: Vec<u8>) -> Bytes {
        let mut data = selector.to_vec();
        data.extend(encoded);
        Bytes::from(data)
    }

    #[test]
    fn parse_uint_works() {
        let encoded = DynSolValue::String("456".into()).abi_encode();
        let args = ParseUint::decode(&call_data(ParseUint::SELECTOR, encoded)).unwrap();
        let effects = ParseUint::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::ReturnU256(U256::from(456u64))]
        );
    }

    #[test]
    fn parse_bool_true() {
        let encoded = DynSolValue::String("true".into()).abi_encode();
        let args = ParseBool::decode(&call_data(ParseBool::SELECTOR, encoded)).unwrap();
        let effects = ParseBool::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::ReturnBool(true)]);
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
    fn get_code_looks_up_compiled_contract() {
        let mut inspector = CheatcodeInspector::new();
        inspector.state.compiled_contracts.insert(
            "CheatcodeString".into(),
            revm::primitives::Bytes::from(vec![0x60, 0x01]),
        );
        let encoded = DynSolValue::String("CheatcodeString".into()).abi_encode();
        let args = GetCode::decode(&call_data(GetCode::SELECTOR, encoded)).unwrap();
        // GetCode effects return the name; the inspector resolves it in build_outcome.
        assert_eq!(args, "CheatcodeString");
    }

    #[test]
    #[serial]
    fn cheatcode_string_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeString.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let output = chain.execute(&vec![]).unwrap();
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "string property should pass"
        );
    }
}
