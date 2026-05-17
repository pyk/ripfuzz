//! Assertion cheatcodes.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::interpreter::CallOutcome;

use crate::chain::cheatcodes::{CheatcodeInspector, dummy_success, revert_outcome};

/// `assertTrue(bool)` — revert if false.
pub const ASSERT_TRUE_SELECTOR: [u8; 4] = [0xc6, 0x81, 0x7c, 0xfd];
/// `assertFalse(bool)` — revert if true.
pub const ASSERT_FALSE_SELECTOR: [u8; 4] = [0x97, 0x11, 0x71, 0x5a];
/// `assertEq(bool, bool)`.
pub const ASSERT_EQ_BOOL_SELECTOR: [u8; 4] = [0x5d, 0x99, 0x5c, 0xaa];
/// `assertEq(uint256, uint256)`.
pub const ASSERT_EQ_UINT_SELECTOR: [u8; 4] = [0x60, 0xb2, 0x8a, 0xb7];
/// `assertEq(int256, int256)`.
pub const ASSERT_EQ_INT_SELECTOR: [u8; 4] = [0xe5, 0xff, 0xc8, 0x1e];
/// `assertEq(address, address)`.
pub const ASSERT_EQ_ADDRESS_SELECTOR: [u8; 4] = [0xd5, 0xfa, 0xda, 0x32];
/// `assertEq(bytes32, bytes32)`.
pub const ASSERT_EQ_BYTES32_SELECTOR: [u8; 4] = [0x6c, 0x9a, 0x2a, 0x4a];
/// `assertEq(string, string)`.
pub const ASSERT_EQ_STRING_SELECTOR: [u8; 4] = [0x0b, 0x34, 0xd8, 0xfc];
/// `assertEq(bytes, bytes)`.
pub const ASSERT_EQ_BYTES_SELECTOR: [u8; 4] = [0xa1, 0xb0, 0xb5, 0x03];
/// `assertNotEq(bool, bool)`.
pub const ASSERT_NOT_EQ_BOOL_SELECTOR: [u8; 4] = [0x98, 0x1b, 0x24, 0xd0];
/// `assertNotEq(uint256, uint256)`.
pub const ASSERT_NOT_EQ_UINT_SELECTOR: [u8; 4] = [0x3e, 0x5e, 0x0e, 0x13];
/// `assertNotEq(int256, int256)`.
pub const ASSERT_NOT_EQ_INT_SELECTOR: [u8; 4] = [0x27, 0x3b, 0x69, 0x12];
/// `assertNotEq(address, address)`.
pub const ASSERT_NOT_EQ_ADDRESS_SELECTOR: [u8; 4] = [0x9a, 0x6a, 0x4c, 0x0b];
/// `assertNotEq(bytes32, bytes32)`.
pub const ASSERT_NOT_EQ_BYTES32_SELECTOR: [u8; 4] = [0x2f, 0x4f, 0x5c, 0xc8];
/// `assertNotEq(string, string)`.
pub const ASSERT_NOT_EQ_STRING_SELECTOR: [u8; 4] = [0x3f, 0x2f, 0x62, 0xf7];
/// `assertNotEq(bytes, bytes)`.
pub const ASSERT_NOT_EQ_BYTES_SELECTOR: [u8; 4] = [0x6d, 0x12, 0xf6, 0xbc];
/// `assertLt(uint256, uint256)`.
pub const ASSERT_LT_UINT_SELECTOR: [u8; 4] = [0x10, 0x10, 0xe8, 0x34];
/// `assertLt(int256, int256)`.
pub const ASSERT_LT_INT_SELECTOR: [u8; 4] = [0xe0, 0x18, 0x67, 0xc9];
/// `assertLe(uint256, uint256)`.
pub const ASSERT_LE_UINT_SELECTOR: [u8; 4] = [0x1c, 0x4e, 0x41, 0xf8];
/// `assertLe(int256, int256)`.
pub const ASSERT_LE_INT_SELECTOR: [u8; 4] = [0x3e, 0x0a, 0x42, 0x44];
/// `assertGt(uint256, uint256)`.
pub const ASSERT_GT_UINT_SELECTOR: [u8; 4] = [0x1c, 0x4e, 0xfa, 0x98];
/// `assertGt(int256, int256)`.
pub const ASSERT_GT_INT_SELECTOR: [u8; 4] = [0xb4, 0x33, 0xd6, 0x68];
/// `assertGe(uint256, uint256)`.
pub const ASSERT_GE_UINT_SELECTOR: [u8; 4] = [0x3e, 0x0b, 0xe2, 0xf5];
/// `assertGe(int256, int256)`.
pub const ASSERT_GE_INT_SELECTOR: [u8; 4] = [0x32, 0x2b, 0x1d, 0x42];

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
        Some(revert_outcome("assertion failed"))
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
        Some(revert_outcome("assertion failed"))
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
        Some(revert_outcome("assertion failed"))
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
        Some(revert_outcome("assertion failed"))
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
        Some(revert_outcome("assertion failed"))
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
    use alloy_primitives::U256;

    use super::*;
    use crate::chain::cheatcodes::CheatcodeInspector;

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
}
