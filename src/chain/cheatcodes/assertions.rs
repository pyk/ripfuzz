//! Assertion cheatcodes.
//!
//! Implements the modern assertion interface (`ensure`, `deny`, `eq`, `ne`,
//! `lt`, `lte`, `gt`, `gte`) with a mandatory `string memory reason`.
//! Every failing assertion produces a revert carrying a clear error message.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

// ---------------------------------------------------------------------------
//  Decoders
// ---------------------------------------------------------------------------

fn decode_bool_and_reason(input: &Bytes) -> Option<(bool, String)> {
    let tuple = DynSolType::Tuple(vec![DynSolType::Bool, DynSolType::String]);
    let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
    match decoded {
        DynSolValue::Tuple(v) if v.len() == 2 => {
            let mut it = v.into_iter();
            let DynSolValue::Bool(b) = it.next()? else {
                return None;
            };
            let DynSolValue::String(reason) = it.next()? else {
                return None;
            };
            Some((b, reason))
        }
        _ => None,
    }
}

fn decode_pair_with_reason(
    input: &Bytes,
    t1: DynSolType,
    t2: DynSolType,
) -> Option<(DynSolValue, DynSolValue, String)> {
    let tuple = DynSolType::Tuple(vec![t1, t2, DynSolType::String]);
    let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
    match decoded {
        DynSolValue::Tuple(v) if v.len() == 3 => {
            let mut it = v.into_iter();
            let a = it.next()?;
            let b = it.next()?;
            let DynSolValue::String(reason) = it.next()? else {
                return None;
            };
            Some((a, b, reason))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
//  Outcome helpers
// ---------------------------------------------------------------------------

/// Build a `CheatcodeEffect::Revert` with a readable assertion-failure message.
fn assertion_failed(reason: &str, detail: &str) -> Vec<CheatcodeEffect> {
    let msg = if reason.is_empty() {
        format!("assertion failed: ({detail})")
    } else {
        format!("assertion failed: {reason} ({detail})")
    };
    vec![CheatcodeEffect::Revert(msg)]
}

fn ok() -> Vec<CheatcodeEffect> {
    vec![]
}

/// Format a [`DynSolValue`] as a human-readable string for error messages.
fn dyn_sol_value_str(v: &DynSolValue) -> String {
    match v {
        DynSolValue::Bool(b) => format!("{b}"),
        DynSolValue::Uint(u, _) => format!("{u}"),
        DynSolValue::Int(i, _) => format!("{i}"),
        DynSolValue::Address(a) => format!("{a}"),
        DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        DynSolValue::String(s) => s.clone(),
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        _ => format!("{v:?}"),
    }
}

// ---------------------------------------------------------------------------
//  Macros
// ---------------------------------------------------------------------------

macro_rules! eq_cheatcode {
    ($name:ident, $selector:expr, $t1:expr, $t2:expr, $pat1:pat, $pat2:pat, $cmp:expr, $detail:expr) => {
        pub struct $name;
        impl Cheatcode for $name {
            type Args = (DynSolValue, DynSolValue, String);
            const SELECTOR: [u8; 4] = $selector;
            fn decode(input: &Bytes) -> Option<Self::Args> {
                decode_pair_with_reason(input, $t1, $t2)
            }
            fn effects((a, b, reason): Self::Args) -> Vec<CheatcodeEffect> {
                let a_ref = &a;
                let b_ref = &b;
                let $pat1 = a_ref else { return vec![] };
                let $pat2 = b_ref else { return vec![] };
                if $cmp {
                    ok()
                } else {
                    let a_str = dyn_sol_value_str(a_ref);
                    let b_str = dyn_sol_value_str(b_ref);
                    let detail = format!($detail, a = a_str, b = b_str);
                    assertion_failed(&reason, &detail)
                }
            }
        }
    };
}

macro_rules! ne_cheatcode {
    ($name:ident, $selector:expr, $t1:expr, $t2:expr, $pat1:pat, $pat2:pat, $cmp:expr, $detail:expr) => {
        pub struct $name;
        impl Cheatcode for $name {
            type Args = (DynSolValue, DynSolValue, String);
            const SELECTOR: [u8; 4] = $selector;
            fn decode(input: &Bytes) -> Option<Self::Args> {
                decode_pair_with_reason(input, $t1, $t2)
            }
            fn effects((a, b, reason): Self::Args) -> Vec<CheatcodeEffect> {
                let a_ref = &a;
                let b_ref = &b;
                let $pat1 = a_ref else { return vec![] };
                let $pat2 = b_ref else { return vec![] };
                if !$cmp {
                    ok()
                } else {
                    let a_str = dyn_sol_value_str(a_ref);
                    let b_str = dyn_sol_value_str(b_ref);
                    let detail = format!($detail, a = a_str, b = b_str);
                    assertion_failed(&reason, &detail)
                }
            }
        }
    };
}

// ---------------------------------------------------------------------------
//  Boolean assertions
// ---------------------------------------------------------------------------

pub struct Ensure;
impl Cheatcode for Ensure {
    type Args = (bool, String);
    const SELECTOR: [u8; 4] = [0x48, 0xa2, 0x1a, 0xf7]; // ensure(bool,string)
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_bool_and_reason(input)
    }
    fn effects((b, reason): Self::Args) -> Vec<CheatcodeEffect> {
        if b {
            ok()
        } else {
            assertion_failed(&reason, "expected true")
        }
    }
}

pub struct Deny;
impl Cheatcode for Deny {
    type Args = (bool, String);
    const SELECTOR: [u8; 4] = [0x78, 0x34, 0x27, 0x6f]; // deny(bool,string)
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_bool_and_reason(input)
    }
    fn effects((b, reason): Self::Args) -> Vec<CheatcodeEffect> {
        if !b {
            ok()
        } else {
            assertion_failed(&reason, "expected false")
        }
    }
}

// ---------------------------------------------------------------------------
//  Equality / inequality (all supported types)
// ---------------------------------------------------------------------------

eq_cheatcode!(
    EqBool,
    [0xdf, 0x7c, 0xd7, 0x7f],
    DynSolType::Bool,
    DynSolType::Bool,
    DynSolValue::Bool(a),
    DynSolValue::Bool(b),
    a == b,
    "{a} != {b}"
);
eq_cheatcode!(
    EqUint,
    [0xbc, 0x8d, 0x43, 0xa8],
    DynSolType::Uint(256),
    DynSolType::Uint(256),
    DynSolValue::Uint(a, _),
    DynSolValue::Uint(b, _),
    a == b,
    "{a} != {b}"
);
eq_cheatcode!(
    EqInt,
    [0x14, 0xec, 0x1c, 0xc6],
    DynSolType::Int(256),
    DynSolType::Int(256),
    DynSolValue::Int(a, _),
    DynSolValue::Int(b, _),
    a == b,
    "{a} != {b}"
);
eq_cheatcode!(
    EqAddress,
    [0x3c, 0xcb, 0x5e, 0x26],
    DynSolType::Address,
    DynSolType::Address,
    DynSolValue::Address(a),
    DynSolValue::Address(b),
    a == b,
    "{a} != {b}"
);
eq_cheatcode!(
    EqBytes32,
    [0xa8, 0xdf, 0x43, 0x01],
    DynSolType::FixedBytes(32),
    DynSolType::FixedBytes(32),
    DynSolValue::FixedBytes(a, _),
    DynSolValue::FixedBytes(b, _),
    a == b,
    "{a} != {b}"
);
eq_cheatcode!(
    EqString,
    [0xdc, 0xe7, 0x99, 0x6c],
    DynSolType::String,
    DynSolType::String,
    DynSolValue::String(a),
    DynSolValue::String(b),
    a == b,
    "{a} != {b}"
);
eq_cheatcode!(
    EqBytes,
    [0x7c, 0xe7, 0xf8, 0x6e],
    DynSolType::Bytes,
    DynSolType::Bytes,
    DynSolValue::Bytes(a),
    DynSolValue::Bytes(b),
    a == b,
    "{a} != {b}"
);

ne_cheatcode!(
    NeBool,
    [0xa9, 0x24, 0x0d, 0x91],
    DynSolType::Bool,
    DynSolType::Bool,
    DynSolValue::Bool(a),
    DynSolValue::Bool(b),
    a == b,
    "{a} == {b}"
);
ne_cheatcode!(
    NeUint,
    [0x59, 0x75, 0x31, 0x8d],
    DynSolType::Uint(256),
    DynSolType::Uint(256),
    DynSolValue::Uint(a, _),
    DynSolValue::Uint(b, _),
    a == b,
    "{a} == {b}"
);
ne_cheatcode!(
    NeInt,
    [0x20, 0xc7, 0x8b, 0x58],
    DynSolType::Int(256),
    DynSolType::Int(256),
    DynSolValue::Int(a, _),
    DynSolValue::Int(b, _),
    a == b,
    "{a} == {b}"
);
ne_cheatcode!(
    NeAddress,
    [0xc8, 0x71, 0x94, 0xf3],
    DynSolType::Address,
    DynSolType::Address,
    DynSolValue::Address(a),
    DynSolValue::Address(b),
    a == b,
    "{a} == {b}"
);
ne_cheatcode!(
    NeBytes32,
    [0x4a, 0x07, 0x25, 0xf9],
    DynSolType::FixedBytes(32),
    DynSolType::FixedBytes(32),
    DynSolValue::FixedBytes(a, _),
    DynSolValue::FixedBytes(b, _),
    a == b,
    "{a} == {b}"
);
ne_cheatcode!(
    NeString,
    [0xd6, 0x38, 0x69, 0xf9],
    DynSolType::String,
    DynSolType::String,
    DynSolValue::String(a),
    DynSolValue::String(b),
    a == b,
    "{a} == {b}"
);
ne_cheatcode!(
    NeBytes,
    [0x35, 0x66, 0xab, 0x09],
    DynSolType::Bytes,
    DynSolType::Bytes,
    DynSolValue::Bytes(a),
    DynSolValue::Bytes(b),
    a == b,
    "{a} == {b}"
);

// ---------------------------------------------------------------------------
//  Ordering (uint256 / int256 only)
// ---------------------------------------------------------------------------

fn cmp_uint_outcome(input: &Bytes, expect_less: bool, expect_equal: bool) -> Vec<CheatcodeEffect> {
    let Some((a, b, reason)) =
        decode_pair_with_reason(input, DynSolType::Uint(256), DynSolType::Uint(256))
    else {
        return vec![];
    };
    let (a, b) = match (&a, &b) {
        (DynSolValue::Uint(a, _), DynSolValue::Uint(b, _)) => (*a, *b),
        _ => return vec![],
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
    let op = if expect_less && expect_equal {
        "<="
    } else if expect_less {
        "<"
    } else if expect_equal {
        ">="
    } else {
        ">"
    };
    if ok {
        vec![]
    } else {
        assertion_failed(&reason, &format!("{a} {op} {b} is false"))
    }
}

fn cmp_int_outcome(input: &Bytes, expect_less: bool, expect_equal: bool) -> Vec<CheatcodeEffect> {
    let Some((a, b, reason)) =
        decode_pair_with_reason(input, DynSolType::Int(256), DynSolType::Int(256))
    else {
        return vec![];
    };
    let (a, b) = match (&a, &b) {
        (DynSolValue::Int(a, _), DynSolValue::Int(b, _)) => (*a, *b),
        _ => return vec![],
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
    let op = if expect_less && expect_equal {
        "<="
    } else if expect_less {
        "<"
    } else if expect_equal {
        ">="
    } else {
        ">"
    };
    if ok {
        vec![]
    } else {
        assertion_failed(&reason, &format!("{a} {op} {b} is false"))
    }
}

macro_rules! cmp_cheatcode {
    ($name:ident, $selector:expr, $fn:ident, $less:expr, $equal:expr) => {
        pub struct $name;
        impl Cheatcode for $name {
            type Args = Bytes;
            const SELECTOR: [u8; 4] = $selector;
            fn decode(input: &Bytes) -> Option<Self::Args> {
                Some(input.clone())
            }
            fn effects(input: Self::Args) -> Vec<CheatcodeEffect> {
                $fn(&input, $less, $equal)
            }
        }
    };
}

cmp_cheatcode!(
    LtUint,
    [0x01, 0xb9, 0xe8, 0x27],
    cmp_uint_outcome,
    true,
    false
);
cmp_cheatcode!(
    LtInt,
    [0x06, 0xf8, 0x23, 0x42],
    cmp_int_outcome,
    true,
    false
);
cmp_cheatcode!(
    LteUint,
    [0xbb, 0x35, 0x03, 0x1a],
    cmp_uint_outcome,
    true,
    true
);
cmp_cheatcode!(
    LteInt,
    [0x1b, 0xa0, 0x39, 0x9b],
    cmp_int_outcome,
    true,
    true
);
cmp_cheatcode!(
    GtUint,
    [0x5c, 0x2b, 0x80, 0xf5],
    cmp_uint_outcome,
    false,
    false
);
cmp_cheatcode!(
    GtInt,
    [0xd2, 0xa5, 0x06, 0x04],
    cmp_int_outcome,
    false,
    false
);
cmp_cheatcode!(
    GteUint,
    [0x84, 0x1e, 0xa1, 0x1c],
    cmp_uint_outcome,
    false,
    true
);
cmp_cheatcode!(
    GteInt,
    [0x3b, 0x6d, 0xdf, 0x03],
    cmp_int_outcome,
    false,
    true
);

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    use alloy_primitives::U256;
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    fn call_data(selector: [u8; 4], encoded: Vec<u8>) -> Bytes {
        let mut data = selector.to_vec();
        data.extend(encoded);
        Bytes::from(data)
    }

    // -----------------------------------------------------------------------
    //  Unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn ensure_decode_happy_path() {
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Bool(true),
            DynSolValue::String("reason".into()),
        ])
        .abi_encode_params();
        let input = call_data(Ensure::SELECTOR, encoded);
        let args = Ensure::decode(&input).unwrap();
        let effects = Ensure::effects(args);
        assert!(effects.is_empty());
    }

    #[test]
    fn ensure_false_produces_revert() {
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Bool(false),
            DynSolValue::String("expected true".into()),
        ])
        .abi_encode_params();
        let input = call_data(Ensure::SELECTOR, encoded);
        let args = Ensure::decode(&input).unwrap();
        let effects = Ensure::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::Revert(
                "assertion failed: expected true (expected true)".into()
            )]
        );
    }

    #[test]
    fn eq_failure_message_includes_reason_and_detail() {
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Uint(U256::from(1), 256),
            DynSolValue::Uint(U256::from(2), 256),
            DynSolValue::String("1 != 2".into()),
        ])
        .abi_encode_params();
        let input = call_data(EqUint::SELECTOR, encoded);
        let args = EqUint::decode(&input).unwrap();
        let effects = EqUint::effects(args);
        let msg = match effects.into_iter().next().unwrap() {
            CheatcodeEffect::Revert(m) => m,
            other => panic!("expected Revert, got {other:?}"),
        };
        assert!(msg.contains("assertion failed"));
        assert!(msg.contains("1 != 2"));
    }

    #[test]
    fn empty_reason_omits_colon() {
        let effects = assertion_failed("", "detail");
        let msg = match effects.into_iter().next().unwrap() {
            CheatcodeEffect::Revert(m) => m,
            other => panic!("expected Revert, got {other:?}"),
        };
        assert_eq!(msg, "assertion failed: (detail)");
    }

    #[test]
    fn deny_true_produces_revert() {
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Bool(true),
            DynSolValue::String("expected false".into()),
        ])
        .abi_encode_params();
        let input = call_data(Deny::SELECTOR, encoded);
        let args = Deny::decode(&input).unwrap();
        let effects = Deny::effects(args);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::Revert(
                "assertion failed: expected false (expected false)".into()
            )]
        );
    }

    #[test]
    fn ne_uint_passes_when_different() {
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Uint(U256::from(1), 256),
            DynSolValue::Uint(U256::from(2), 256),
            DynSolValue::String("should differ".into()),
        ])
        .abi_encode_params();
        let input = call_data(NeUint::SELECTOR, encoded);
        let args = NeUint::decode(&input).unwrap();
        let effects = NeUint::effects(args);
        assert!(effects.is_empty());
    }

    #[test]
    fn lt_uint_fails_when_greater() {
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Uint(U256::from(100), 256),
            DynSolValue::Uint(U256::from(42), 256),
            DynSolValue::String("100 < 42".into()),
        ])
        .abi_encode_params();
        let input = call_data(LtUint::SELECTOR, encoded);
        let args = LtUint::decode(&input).unwrap();
        let effects = LtUint::effects(args);
        let msg = match effects.into_iter().next().unwrap() {
            CheatcodeEffect::Revert(m) => m,
            other => panic!("expected Revert, got {other:?}"),
        };
        assert!(msg.contains("assertion failed"));
        assert!(msg.contains("100 < 42 is false"));
    }

    // -----------------------------------------------------------------------
    //  Integration tests
    // -----------------------------------------------------------------------

    #[test]
    #[serial]
    fn ensure_true_passes() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xa0, 0x17, 0xfb, 0x20]; // call_ensure_true()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "ensure(true) should pass silently");
    }

    #[test]
    #[serial]
    fn ensure_false_reverts() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xdb, 0x6c, 0xef, 0x2d]; // call_ensure_false_should_revert()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(!out.all_ok, "ensure(false) should revert");
    }

    #[test]
    #[serial]
    fn deny_false_passes() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0x5a, 0x17, 0x4c, 0x25]; // call_deny_false()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "deny(false) should pass silently");
    }

    #[test]
    #[serial]
    fn deny_true_reverts() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xbe, 0x7c, 0xfc, 0x75]; // call_deny_true_should_revert()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(!out.all_ok, "deny(true) should revert");
    }

    #[test]
    #[serial]
    fn eq_uint_passes() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xc6, 0x03, 0x17, 0xe1]; // call_eq_uint()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "eq(uint,uint) should pass");
    }

    #[test]
    #[serial]
    fn eq_uint_fails() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xaa, 0x9e, 0xec, 0x56]; // call_eq_uint_fail()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(!out.all_ok, "eq(1,2) should fail");
    }

    #[test]
    #[serial]
    fn ne_uint_passes() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xed, 0xdf, 0xba, 0xe3]; // call_ne_uint()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "ne(1,2) should pass");
    }

    #[test]
    #[serial]
    fn lt_int_passes() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0x7a, 0xfe, 0x82, 0xb9]; // call_lt_int()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "lt(-2,-1) should pass");
    }

    #[test]
    #[serial]
    fn lte_uint_boundary() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xdb, 0x4f, 0x7c, 0x95]; // call_lte_uint()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "lte(2,2) should pass");
    }

    #[test]
    #[serial]
    fn gte_int_boundary() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xb5, 0x7c, 0x1e, 0x7a]; // call_gte_int()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "gte(-1,-1) should pass");
    }

    #[test]
    #[serial]
    fn setup_ensure_persists() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xd8, 0x40, 0x0e, 0xe9]; // property_setup_ensure_passed()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        let prop = out
            .property_results
            .iter()
            .find(|p| p.name == "property_setup_ensure_passed")
            .expect("property should exist");
        assert!(prop.passed, "setUp ensure should have succeeded");
    }

    #[test]
    #[serial]
    fn setup_assertion_failure_aborts() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertionsSetupFail.sol"),
        )
        .unwrap();
        let result = Chain::initialize(&artifact).unwrap().setup();
        assert!(
            result.is_err(),
            "setUp assertion failure should abort campaign init"
        );
    }

    #[test]
    #[serial]
    fn same_sequence_aborts_on_fail() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel_fail: [u8; 4] = [0x72, 0x88, 0xa2, 0x39]; // call_record_then_fail()
        let sel_read: [u8; 4] = [0xd3, 0xb1, 0x54, 0xbc]; // call_read_recorded()
        let out = chain
            .execute(&[
                Call {
                    selector: sel_fail,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
                Call {
                    selector: sel_read,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
            ])
            .unwrap();
        assert!(!out.all_ok, "sequence should abort on assertion failure");
    }

    #[test]
    #[serial]
    fn revert_rolls_back_state() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel_fail: [u8; 4] = [0x72, 0x88, 0xa2, 0x39]; // call_record_then_fail()
        let out = chain
            .execute(&[Call {
                selector: sel_fail,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        let prop = out
            .property_results
            .iter()
            .find(|p| p.name == "property_recorded_after_failure")
            .expect("property should exist");
        assert!(
            prop.passed,
            "revert should roll back state mutations from failing call"
        );
    }

    #[test]
    #[serial]
    fn cross_sequence_isolation() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain_a = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel_set: [u8; 4] = [0x20, 0x1c, 0xc9, 0xbd]; // call_set_recorded(uint256)
        let args = DynSolValue::Uint(U256::from(999), 256).abi_encode();
        let out_a = chain_a
            .execute(&[Call {
                selector: sel_set,
                args,
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out_a.all_ok, "sequence A should succeed");

        let chain_b = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel_prop: [u8; 4] = [0x88, 0x32, 0xfb, 0x28]; // property_cross_sequence_isolation()
        let out_b = chain_b
            .execute(&[Call {
                selector: sel_prop,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        let prop = out_b
            .property_results
            .iter()
            .find(|p| p.name == "property_cross_sequence_isolation")
            .expect("property should exist");
        assert!(prop.passed, "cross-sequence isolation should hold");
    }

    #[test]
    #[serial]
    fn eq_zero() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xe0, 0x97, 0x7a, 0x6a]; // call_eq_zero()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "eq(0,0) should pass");
    }

    #[test]
    #[serial]
    fn eq_max_uint256() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0x7a, 0x23, 0xa7, 0x0c]; // call_eq_max()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "eq(max,max) should pass");
    }

    #[test]
    #[serial]
    fn eq_empty_string() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0xf3, 0xbb, 0xec, 0x28]; // call_eq_empty_string()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "eq(\"\",\"\") should pass");
    }

    #[test]
    #[serial]
    fn eq_empty_bytes() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0x98, 0xf6, 0x47, 0xfc]; // call_eq_empty_bytes()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "eq(bytes(\"\"),bytes(\"\")) should pass");
    }

    #[test]
    #[serial]
    fn lt_zero_vs_max() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0x68, 0x0c, 0xc7, 0xad]; // call_lt_uint_zero_vs_max()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "lt(0,max) should pass");
    }

    #[test]
    #[serial]
    fn gt_int_min_vs_max() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let sel: [u8; 4] = [0x7f, 0xa8, 0xf3, 0x15]; // call_gt_int_min_vs_max()
        let out = chain
            .execute(&[Call {
                selector: sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(out.all_ok, "gt(max,min) should pass");
    }

    #[test]
    #[serial]
    fn no_side_effect_leak() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAssertions.sol"),
        )
        .unwrap();
        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();

        // Run a mix of happy-path assertion calls.
        let sels: [[u8; 4]; 13] = [
            [0xa0, 0x17, 0xfb, 0x20], // call_ensure_true
            [0x5a, 0x17, 0x4c, 0x25], // call_deny_false
            [0xc6, 0x03, 0x17, 0xe1], // call_eq_uint
            [0x61, 0xf6, 0x78, 0x9c], // call_eq_int
            [0xd1, 0x36, 0xba, 0xfb], // call_eq_bool
            [0x59, 0x53, 0x24, 0xf2], // call_eq_address
            [0x5b, 0x93, 0x80, 0xda], // call_eq_bytes32
            [0xe4, 0x3b, 0x31, 0x2c], // call_eq_string
            [0x05, 0x4e, 0x57, 0x45], // call_eq_bytes
            [0xed, 0xdf, 0xba, 0xe3], // call_ne_uint
            [0xa3, 0xb3, 0x20, 0x08], // call_ne_int
            [0x20, 0x87, 0xea, 0x29], // call_lt_uint
            [0xc9, 0x3a, 0xc4, 0x63], // call_gt_uint
        ];
        let calls: Vec<Call> = sels
            .iter()
            .map(|sel| Call {
                selector: *sel,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            })
            .collect();
        let out = chain.execute(&calls).unwrap();
        assert!(out.all_ok, "all happy calls should succeed");

        let sel_prop: [u8; 4] = [0xad, 0xce, 0xc7, 0x35]; // property_no_side_effect_leak()
        let out_prop = chain
            .execute(&[Call {
                selector: sel_prop,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        let prop = out_prop
            .property_results
            .iter()
            .find(|p| p.name == "property_no_side_effect_leak")
            .expect("property should exist");
        assert!(prop.passed, "assertions should not leak side effects");
    }
}
