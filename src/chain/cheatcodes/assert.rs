//! Assertion cheatcodes.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::interpreter::CallOutcome;

use crate::chain::cheatcodes::{CheatcodeInspector, dummy_success, panic_outcome};

/// `assertTrue(bool)` — revert if false.
pub const ASSERT_TRUE_SELECTOR: [u8; 4] = [0x0c, 0x9f, 0xd5, 0x81];
/// `assertFalse(bool)` — revert if true.
pub const ASSERT_FALSE_SELECTOR: [u8; 4] = [0xa5, 0x98, 0x28, 0x85];
/// `assertEq(bool, bool)`.
pub const ASSERT_EQ_BOOL_SELECTOR: [u8; 4] = [0xf7, 0xfe, 0x34, 0x77];
/// `assertEq(uint256, uint256)`.
pub const ASSERT_EQ_UINT_SELECTOR: [u8; 4] = [0x98, 0x29, 0x6c, 0x54];
/// `assertEq(int256, int256)`.
pub const ASSERT_EQ_INT_SELECTOR: [u8; 4] = [0xfe, 0x74, 0xf0, 0x5b];
/// `assertEq(address, address)`.
pub const ASSERT_EQ_ADDRESS_SELECTOR: [u8; 4] = [0x51, 0x53, 0x61, 0xf6];
/// `assertEq(bytes32, bytes32)`.
pub const ASSERT_EQ_BYTES32_SELECTOR: [u8; 4] = [0x7c, 0x84, 0xc6, 0x9b];
/// `assertEq(string, string)`.
pub const ASSERT_EQ_STRING_SELECTOR: [u8; 4] = [0xf3, 0x20, 0xd9, 0x63];
/// `assertEq(bytes, bytes)`.
pub const ASSERT_EQ_BYTES_SELECTOR: [u8; 4] = [0x97, 0x62, 0x46, 0x31];
/// `assertNotEq(bool, bool)`.
pub const ASSERT_NOT_EQ_BOOL_SELECTOR: [u8; 4] = [0x23, 0x6e, 0x4d, 0x66];
/// `assertNotEq(uint256, uint256)`.
pub const ASSERT_NOT_EQ_UINT_SELECTOR: [u8; 4] = [0xb7, 0x90, 0x93, 0x20];
/// `assertNotEq(int256, int256)`.
pub const ASSERT_NOT_EQ_INT_SELECTOR: [u8; 4] = [0xf4, 0xc0, 0x04, 0xe3];
/// `assertNotEq(address, address)`.
pub const ASSERT_NOT_EQ_ADDRESS_SELECTOR: [u8; 4] = [0xb1, 0x2e, 0x16, 0x94];
/// `assertNotEq(bytes32, bytes32)`.
pub const ASSERT_NOT_EQ_BYTES32_SELECTOR: [u8; 4] = [0x89, 0x8e, 0x83, 0xfc];
/// `assertNotEq(string, string)`.
pub const ASSERT_NOT_EQ_STRING_SELECTOR: [u8; 4] = [0x6a, 0x82, 0x37, 0xb3];
/// `assertNotEq(bytes, bytes)`.
pub const ASSERT_NOT_EQ_BYTES_SELECTOR: [u8; 4] = [0x3c, 0xf7, 0x8e, 0x28];
/// `assertLt(uint256, uint256)`.
pub const ASSERT_LT_UINT_SELECTOR: [u8; 4] = [0xb1, 0x2f, 0xc0, 0x05];
/// `assertLt(int256, int256)`.
pub const ASSERT_LT_INT_SELECTOR: [u8; 4] = [0x3e, 0x91, 0x40, 0x80];
/// `assertLe(uint256, uint256)`.
pub const ASSERT_LE_UINT_SELECTOR: [u8; 4] = [0x84, 0x66, 0xf4, 0x15];
/// `assertLe(int256, int256)`.
pub const ASSERT_LE_INT_SELECTOR: [u8; 4] = [0x95, 0xfd, 0x15, 0x4e];
/// `assertGt(uint256, uint256)`.
pub const ASSERT_GT_UINT_SELECTOR: [u8; 4] = [0xdb, 0x07, 0xfc, 0xd2];
/// `assertGt(int256, int256)`.
pub const ASSERT_GT_INT_SELECTOR: [u8; 4] = [0x5a, 0x36, 0x2d, 0x45];
/// `assertGe(uint256, uint256)`.
pub const ASSERT_GE_UINT_SELECTOR: [u8; 4] = [0xa8, 0xd4, 0xd1, 0xd9];
/// `assertGe(int256, int256)`.
pub const ASSERT_GE_INT_SELECTOR: [u8; 4] = [0x0a, 0x30, 0xb7, 0x71];

fn decode_pair(
    input: &revm::primitives::Bytes,
    t1: DynSolType,
    t2: DynSolType,
) -> Option<(DynSolValue, DynSolValue)> {
    let tuple = DynSolType::Tuple(vec![t1, t2]);
    let decoded = match tuple.abi_decode_params(&input[4..]) {
        Ok(v) => v,
        Err(_) => return None,
    };
    match decoded {
        DynSolValue::Tuple(v) if v.len() == 2 => {
            let mut it = v.into_iter();
            Some((it.next()?, it.next()?))
        }
        _ => None,
    }
}

fn decode_single_bool(input: &revm::primitives::Bytes) -> Option<bool> {
    let tuple = DynSolType::Tuple(vec![DynSolType::Bool]);
    let decoded = match tuple.abi_decode_params(&input[4..]) {
        Ok(v) => v,
        Err(_) => return None,
    };
    match decoded {
        DynSolValue::Tuple(v) => match v.into_iter().next()? {
            DynSolValue::Bool(b) => Some(b),
            _ => None,
        },
        _ => None,
    }
}

fn eq_outcome(equal_expected: bool, actual_equal: bool) -> Option<CallOutcome> {
    if actual_equal == equal_expected {
        Some(dummy_success())
    } else {
        Some(panic_outcome())
    }
}

pub fn handle_assert_true(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let b = decode_single_bool(input)?;
    if b {
        Some(dummy_success())
    } else {
        Some(panic_outcome())
    }
}

pub fn handle_assert_false(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let b = decode_single_bool(input)?;
    if !b {
        Some(dummy_success())
    } else {
        Some(panic_outcome())
    }
}

pub fn handle_assert_eq_bool(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Bool, DynSolType::Bool)?;
    match (&a, &b) {
        (DynSolValue::Bool(a), DynSolValue::Bool(b)) => eq_outcome(true, a == b),
        _ => None,
    }
}

pub fn handle_assert_eq_uint(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Uint(256), DynSolType::Uint(256))?;
    match (&a, &b) {
        (DynSolValue::Uint(a, _), DynSolValue::Uint(b, _)) => eq_outcome(true, a == b),
        _ => None,
    }
}

pub fn handle_assert_eq_int(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Int(256), DynSolType::Int(256))?;
    match (&a, &b) {
        (DynSolValue::Int(a, _), DynSolValue::Int(b, _)) => eq_outcome(true, a == b),
        _ => None,
    }
}

pub fn handle_assert_eq_address(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Address, DynSolType::Address)?;
    match (&a, &b) {
        (DynSolValue::Address(a), DynSolValue::Address(b)) => eq_outcome(true, a == b),
        _ => None,
    }
}

pub fn handle_assert_eq_bytes32(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(
        input,
        DynSolType::FixedBytes(32),
        DynSolType::FixedBytes(32),
    )?;
    match (&a, &b) {
        (DynSolValue::FixedBytes(a, _), DynSolValue::FixedBytes(b, _)) => eq_outcome(true, a == b),
        _ => None,
    }
}

pub fn handle_assert_eq_string(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::String, DynSolType::String)?;
    match (&a, &b) {
        (DynSolValue::String(a), DynSolValue::String(b)) => eq_outcome(true, a == b),
        _ => None,
    }
}

pub fn handle_assert_eq_bytes(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Bytes, DynSolType::Bytes)?;
    match (&a, &b) {
        (DynSolValue::Bytes(a), DynSolValue::Bytes(b)) => eq_outcome(true, a == b),
        _ => None,
    }
}

pub fn handle_assert_not_eq_bool(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Bool, DynSolType::Bool)?;
    match (&a, &b) {
        (DynSolValue::Bool(a), DynSolValue::Bool(b)) => eq_outcome(false, a == b),
        _ => None,
    }
}

pub fn handle_assert_not_eq_uint(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Uint(256), DynSolType::Uint(256))?;
    match (&a, &b) {
        (DynSolValue::Uint(a, _), DynSolValue::Uint(b, _)) => eq_outcome(false, a == b),
        _ => None,
    }
}

pub fn handle_assert_not_eq_int(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Int(256), DynSolType::Int(256))?;
    match (&a, &b) {
        (DynSolValue::Int(a, _), DynSolValue::Int(b, _)) => eq_outcome(false, a == b),
        _ => None,
    }
}

pub fn handle_assert_not_eq_address(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Address, DynSolType::Address)?;
    match (&a, &b) {
        (DynSolValue::Address(a), DynSolValue::Address(b)) => eq_outcome(false, a == b),
        _ => None,
    }
}

pub fn handle_assert_not_eq_bytes32(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(
        input,
        DynSolType::FixedBytes(32),
        DynSolType::FixedBytes(32),
    )?;
    match (&a, &b) {
        (DynSolValue::FixedBytes(a, _), DynSolValue::FixedBytes(b, _)) => eq_outcome(false, a == b),
        _ => None,
    }
}

pub fn handle_assert_not_eq_string(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::String, DynSolType::String)?;
    match (&a, &b) {
        (DynSolValue::String(a), DynSolValue::String(b)) => eq_outcome(false, a == b),
        _ => None,
    }
}

pub fn handle_assert_not_eq_bytes(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Bytes, DynSolType::Bytes)?;
    match (&a, &b) {
        (DynSolValue::Bytes(a), DynSolValue::Bytes(b)) => eq_outcome(false, a == b),
        _ => None,
    }
}

fn cmp_uint_outcome(
    input: &revm::primitives::Bytes,
    expect_less: bool,
    expect_equal: bool,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Uint(256), DynSolType::Uint(256))?;
    let (a, b) = match (&a, &b) {
        (DynSolValue::Uint(a, _), DynSolValue::Uint(b, _)) => (*a, *b),
        _ => return None,
    };
    let ok = if expect_less && expect_equal {
        a <= b
    } else if expect_less {
        a < b
    } else if expect_equal {
        a >= b
    } else {
        a > b
    };
    if ok {
        Some(dummy_success())
    } else {
        Some(panic_outcome())
    }
}

fn cmp_int_outcome(
    input: &revm::primitives::Bytes,
    expect_less: bool,
    expect_equal: bool,
) -> Option<CallOutcome> {
    let (a, b) = decode_pair(input, DynSolType::Int(256), DynSolType::Int(256))?;
    let (a, b) = match (&a, &b) {
        (DynSolValue::Int(a, _), DynSolValue::Int(b, _)) => (*a, *b),
        _ => return None,
    };
    let ok = if expect_less && expect_equal {
        a <= b
    } else if expect_less {
        a < b
    } else if expect_equal {
        a >= b
    } else {
        a > b
    };
    if ok {
        Some(dummy_success())
    } else {
        Some(panic_outcome())
    }
}

pub fn handle_assert_lt_uint(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    cmp_uint_outcome(input, true, false)
}

pub fn handle_assert_lt_int(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    cmp_int_outcome(input, true, false)
}

pub fn handle_assert_le_uint(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    cmp_uint_outcome(input, true, true)
}

pub fn handle_assert_le_int(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    cmp_int_outcome(input, true, true)
}

pub fn handle_assert_gt_uint(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    cmp_uint_outcome(input, false, false)
}

pub fn handle_assert_gt_int(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    cmp_int_outcome(input, false, false)
}

pub fn handle_assert_ge_uint(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    cmp_uint_outcome(input, false, true)
}

pub fn handle_assert_ge_int(
    _inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    cmp_int_outcome(input, false, true)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use alloy_primitives::U256;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::CheatcodeInspector;
    use crate::contract;
    use crate::corpus::Call;

    fn call_data(selector: [u8; 4], encoded: Vec<u8>) -> revm::primitives::Bytes {
        let mut data = selector.to_vec();
        data.extend(encoded);
        revm::primitives::Bytes::from(data)
    }

    #[test]
    fn assert_true_passes() {
        let mut inspector = CheatcodeInspector::new();
        let encoded = DynSolValue::Bool(true).abi_encode();
        let result = handle_assert_true(&mut inspector, &call_data(ASSERT_TRUE_SELECTOR, encoded));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().result.result,
            revm::interpreter::InstructionResult::Stop
        );
    }

    #[test]
    fn assert_true_fails() {
        let mut inspector = CheatcodeInspector::new();
        let encoded = DynSolValue::Bool(false).abi_encode();
        let result = handle_assert_true(&mut inspector, &call_data(ASSERT_TRUE_SELECTOR, encoded));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().result.result,
            revm::interpreter::InstructionResult::Revert
        );
    }

    #[test]
    fn assert_eq_uint_passes() {
        let mut inspector = CheatcodeInspector::new();
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Uint(U256::from(42u64), 256),
            DynSolValue::Uint(U256::from(42u64), 256),
        ])
        .abi_encode();
        let result =
            handle_assert_eq_uint(&mut inspector, &call_data(ASSERT_EQ_UINT_SELECTOR, encoded));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().result.result,
            revm::interpreter::InstructionResult::Stop
        );
    }

    #[test]
    fn assert_lt_uint_fails_when_greater() {
        let mut inspector = CheatcodeInspector::new();
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Uint(U256::from(100u64), 256),
            DynSolValue::Uint(U256::from(42u64), 256),
        ])
        .abi_encode();
        let result =
            handle_assert_lt_uint(&mut inspector, &call_data(ASSERT_LT_UINT_SELECTOR, encoded));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().result.result,
            revm::interpreter::InstructionResult::Revert
        );
    }

    #[test]
    fn assert_false_passes() {
        let mut inspector = CheatcodeInspector::new();
        let encoded = DynSolValue::Bool(false).abi_encode();
        let result =
            handle_assert_false(&mut inspector, &call_data(ASSERT_FALSE_SELECTOR, encoded));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().result.result,
            revm::interpreter::InstructionResult::Stop
        );
    }

    #[test]
    fn assert_false_fails() {
        let mut inspector = CheatcodeInspector::new();
        let encoded = DynSolValue::Bool(true).abi_encode();
        let result =
            handle_assert_false(&mut inspector, &call_data(ASSERT_FALSE_SELECTOR, encoded));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().result.result,
            revm::interpreter::InstructionResult::Revert
        );
    }

    #[test]
    fn assert_panic_encoding_matches_solidity() {
        let result = panic_outcome();
        let out = result.result.output;
        assert_eq!(&out[..4], &[0x4e, 0x48, 0x7b, 0x71]); // Panic(uint256)
        assert_eq!(&out[4..35], &[0u8; 31]); // padded uint256(1)
        assert_eq!(out[35], 0x01);
    }

    #[test]
    fn cheatcode_assert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssert.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel_true: [u8; 4] = [0xd6, 0xb8, 0xdf, 0x1b]; // action_assert_true()
        let sel_eq: [u8; 4] = [0xd3, 0xe5, 0x18, 0xfd]; // action_assert_eq_uint()
        let sel_lt: [u8; 4] = [0xdb, 0x0e, 0x9e, 0x0c]; // action_assert_lt()

        for sel in [sel_true, sel_eq, sel_lt] {
            let out = chain
                .execute(&vec![Call {
                    selector: sel,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }])
                .unwrap();
            eprintln!("sel={:02x?} all_ok={}", sel, out.all_ok);
            assert!(out.all_ok, "each assert action should succeed");
        }
        let output = chain.execute(&vec![]).unwrap();
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "assert property should pass"
        );
    }
}
