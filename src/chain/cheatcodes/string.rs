//! String / type conversion cheatcodes.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, I256, U256};
use revm::interpreter::CallOutcome;

use crate::chain::cheatcodes::{
    CheatcodeInspector, revert_outcome, success_bool_outcome, success_bytes_outcome,
    success_u256_outcome,
};

/// `toString(address)` returns `string`.
pub const TO_STRING_ADDRESS_SELECTOR: [u8; 4] = [0x2f, 0xbe, 0x31, 0xfa];
/// `toString(bool)` returns `string`.
pub const TO_STRING_BOOL_SELECTOR: [u8; 4] = [0x4f, 0x0c, 0xb2, 0x59];
/// `toString(uint256)` returns `string`.
pub const TO_STRING_UINT_SELECTOR: [u8; 4] = [0xbe, 0x68, 0x0e, 0x08];
/// `toString(int256)` returns `string`.
pub const TO_STRING_INT_SELECTOR: [u8; 4] = [0x65, 0xd2, 0x9c, 0xcf];
/// `toString(bytes32)` returns `string`.
pub const TO_STRING_BYTES32_SELECTOR: [u8; 4] = [0x3b, 0x53, 0x3f, 0x4a];
/// `toString(bytes)` returns `string`.
pub const TO_STRING_BYTES_SELECTOR: [u8; 4] = [0x4f, 0x49, 0x36, 0x7b];
/// `parseUint(string)` returns `uint256`.
pub const PARSE_UINT_SELECTOR: [u8; 4] = [0x2e, 0x33, 0xd0, 0x57];
/// `parseInt(string)` returns `int256`.
pub const PARSE_INT_SELECTOR: [u8; 4] = [0x6c, 0x4c, 0x0f, 0x6c];
/// `parseBool(string)` returns `bool`.
pub const PARSE_BOOL_SELECTOR: [u8; 4] = [0x9d, 0xd3, 0x21, 0x6e];
/// `parseAddress(string)` returns `address`.
pub const PARSE_ADDRESS_SELECTOR: [u8; 4] = [0x72, 0xeb, 0x5f, 0x63];
/// `parseBytes(string)` returns `bytes`.
pub const PARSE_BYTES_SELECTOR: [u8; 4] = [0xf0, 0x60, 0x65, 0x81];
/// `parseBytes32(string)` returns `bytes32`.
pub const PARSE_BYTES32_SELECTOR: [u8; 4] = [0xd3, 0xa9, 0x15, 0x96];
/// `getCode(string)` returns `bytes`.
pub const GET_CODE_SELECTOR: [u8; 4] = [0x98, 0xe0, 0xc3, 0xfe];

fn encode_string(s: &str) -> Vec<u8> {
    DynSolValue::String(s.into()).abi_encode()
}

fn decode_single(input: &revm::primitives::Bytes, t: DynSolType) -> Option<DynSolValue> {
    let tuple = DynSolType::Tuple(vec![t]);
    let decoded = match tuple.abi_decode_params(&input[4..]) {
        Ok(v) => v,
        Err(_) => return None,
    };
    match decoded {
        DynSolValue::Tuple(v) => v.into_iter().next(),
        _ => None,
    }
}

pub fn handle_to_string_address(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::Address)?;
    let DynSolValue::Address(addr) = val else {
        return None;
    };
    Some(success_bytes_outcome(encode_string(&format!("{}", addr))))
}

pub fn handle_to_string_bool(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::Bool)?;
    let DynSolValue::Bool(b) = val else {
        return None;
    };
    Some(success_bytes_outcome(encode_string(&format!("{}", b))))
}

pub fn handle_to_string_uint(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::Uint(256))?;
    let DynSolValue::Uint(u, _) = val else {
        return None;
    };
    Some(success_bytes_outcome(encode_string(&format!("{}", u))))
}

pub fn handle_to_string_int(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::Int(256))?;
    let DynSolValue::Int(i, _) = val else {
        return None;
    };
    Some(success_bytes_outcome(encode_string(&format!("{}", i))))
}

pub fn handle_to_string_bytes32(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::FixedBytes(32))?;
    let DynSolValue::FixedBytes(b, _) = val else {
        return None;
    };
    Some(success_bytes_outcome(encode_string(&format!(
        "0x{}",
        hex::encode(b)
    ))))
}

pub fn handle_to_string_bytes(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::Bytes)?;
    let DynSolValue::Bytes(b) = val else {
        return None;
    };
    Some(success_bytes_outcome(encode_string(&format!(
        "0x{}",
        hex::encode(b)
    ))))
}

pub fn handle_parse_uint(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::String)?;
    let DynSolValue::String(s) = val else {
        return None;
    };
    let u = match U256::from_str_radix(&s, 10) {
        Ok(v) => v,
        Err(_) => return None,
    };
    Some(success_u256_outcome(u))
}

pub fn handle_parse_int(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::String)?;
    let DynSolValue::String(s) = val else {
        return None;
    };
    let negative = s.starts_with('-');
    let mag_str = if negative { &s[1..] } else { &s };
    let mag = match U256::from_str_radix(mag_str, 10) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let sign = if negative {
        alloy_primitives::Sign::Negative
    } else {
        alloy_primitives::Sign::Positive
    };
    let i = I256::checked_from_sign_and_abs(sign, mag)?;
    Some(success_bytes_outcome(DynSolValue::Int(i, 256).abi_encode()))
}

pub fn handle_parse_bool(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::String)?;
    let DynSolValue::String(s) = val else {
        return None;
    };
    let b = s.trim().eq_ignore_ascii_case("true");
    Some(success_bool_outcome(b))
}

pub fn handle_parse_address(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::String)?;
    let DynSolValue::String(s) = val else {
        return None;
    };
    let cleaned = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    let bytes = match hex::decode(cleaned) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if bytes.len() != 20 {
        return None;
    }
    let addr = Address::from_slice(&bytes);
    let mut out = vec![0u8; 32];
    out[12..32].copy_from_slice(addr.as_slice());
    Some(success_bytes_outcome(out))
}

pub fn handle_parse_bytes(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::String)?;
    let DynSolValue::String(s) = val else {
        return None;
    };
    let cleaned = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    let bytes = match hex::decode(cleaned) {
        Ok(v) => v,
        Err(_) => return None,
    };
    Some(success_bytes_outcome(
        DynSolValue::Bytes(bytes).abi_encode(),
    ))
}

pub fn handle_parse_bytes32(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let val = decode_single(input, DynSolType::String)?;
    let DynSolValue::String(s) = val else {
        return None;
    };
    let cleaned = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    let bytes = match hex::decode(cleaned) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if bytes.len() != 32 {
        return None;
    }
    Some(success_bytes_outcome(bytes))
}

pub fn handle_get_code(
    _inspector: &mut CheatcodeInspector,
    _input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    Some(revert_outcome("getCode not yet supported"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::cheatcodes::CheatcodeInspector;

    fn call_data(selector: [u8; 4], encoded: Vec<u8>) -> revm::primitives::Bytes {
        let mut data = selector.to_vec();
        data.extend(encoded);
        revm::primitives::Bytes::from(data)
    }

    #[test]
    fn to_string_uint_works() {
        let mut inspector = CheatcodeInspector::new();
        let encoded = DynSolValue::Uint(U256::from(123u64), 256).abi_encode();
        let result =
            handle_to_string_uint(&mut inspector, &call_data(TO_STRING_UINT_SELECTOR, encoded));
        assert!(result.is_some());
        let out = result.unwrap().result.output;
        let decoded = DynSolType::String.abi_decode_params(&out).unwrap();
        assert_eq!(decoded, DynSolValue::String("123".into()));
    }

    #[test]
    fn parse_uint_works() {
        let mut inspector = CheatcodeInspector::new();
        let encoded = DynSolValue::String("456".into()).abi_encode();
        let result = handle_parse_uint(&mut inspector, &call_data(PARSE_UINT_SELECTOR, encoded));
        assert!(result.is_some());
        let out = result.unwrap().result.output;
        assert_eq!(U256::from_be_slice(&out), U256::from(456));
    }

    #[test]
    fn parse_bool_true() {
        let mut inspector = CheatcodeInspector::new();
        let encoded = DynSolValue::String("true".into()).abi_encode();
        let result = handle_parse_bool(&mut inspector, &call_data(PARSE_BOOL_SELECTOR, encoded));
        assert!(result.is_some());
        assert_eq!(result.unwrap().result.output[31], 1);
    }

    #[test]
    fn parse_address_works() {
        let mut inspector = CheatcodeInspector::new();
        let encoded =
            DynSolValue::String("0x71C7656EC7ab88b098defB751B7401B5f6d8976F".into()).abi_encode();
        let result =
            handle_parse_address(&mut inspector, &call_data(PARSE_ADDRESS_SELECTOR, encoded));
        assert!(result.is_some());
        let out = result.unwrap().result.output;
        assert_eq!(
            &out[12..32],
            hex::decode("71c7656ec7ab88b098defb751b7401b5f6d8976f").unwrap()
        );
    }
}
