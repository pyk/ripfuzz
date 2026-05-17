//! Assertion cheatcodes.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

fn decode_pair(
    input: &Bytes,
    t1: DynSolType,
    t2: DynSolType,
) -> Option<(DynSolValue, DynSolValue)> {
    let tuple = DynSolType::Tuple(vec![t1, t2]);
    let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
    match decoded {
        DynSolValue::Tuple(v) if v.len() == 2 => {
            let mut it = v.into_iter();
            Some((it.next()?, it.next()?))
        }
        _ => None,
    }
}

fn decode_single_bool(input: &Bytes) -> Option<bool> {
    let tuple = DynSolType::Tuple(vec![DynSolType::Bool]);
    let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
    match decoded {
        DynSolValue::Tuple(v) => match v.into_iter().next()? {
            DynSolValue::Bool(b) => Some(b),
            _ => None,
        },
        _ => None,
    }
}

fn eq_outcome(equal_expected: bool, actual_equal: bool) -> Vec<CheatcodeEffect> {
    if actual_equal == equal_expected {
        vec![]
    } else {
        vec![CheatcodeEffect::Panic]
    }
}

macro_rules! assert_eq_cheatcode {
    ($name:ident, $selector:expr, $t1:expr, $t2:expr, $pat1:pat, $pat2:pat, $cmp:expr) => {
        pub struct $name;
        impl Cheatcode for $name {
            type Args = (DynSolValue, DynSolValue);
            const SELECTOR: [u8; 4] = $selector;
            fn decode(input: &Bytes) -> Option<Self::Args> {
                decode_pair(input, $t1, $t2)
            }
            fn effects((a, b): Self::Args) -> Vec<CheatcodeEffect> {
                let $pat1 = &a else { return vec![] };
                let $pat2 = &b else { return vec![] };
                eq_outcome(true, $cmp)
            }
        }
    };
}

macro_rules! assert_not_eq_cheatcode {
    ($name:ident, $selector:expr, $t1:expr, $t2:expr, $pat1:pat, $pat2:pat, $cmp:expr) => {
        pub struct $name;
        impl Cheatcode for $name {
            type Args = (DynSolValue, DynSolValue);
            const SELECTOR: [u8; 4] = $selector;
            fn decode(input: &Bytes) -> Option<Self::Args> {
                decode_pair(input, $t1, $t2)
            }
            fn effects((a, b): Self::Args) -> Vec<CheatcodeEffect> {
                let $pat1 = &a else { return vec![] };
                let $pat2 = &b else { return vec![] };
                eq_outcome(false, $cmp)
            }
        }
    };
}

assert_eq_cheatcode!(
    AssertEqBool,
    [0xf7, 0xfe, 0x34, 0x77],
    DynSolType::Bool,
    DynSolType::Bool,
    DynSolValue::Bool(a),
    DynSolValue::Bool(b),
    a == b
);
assert_eq_cheatcode!(
    AssertEqUint,
    [0x98, 0x29, 0x6c, 0x54],
    DynSolType::Uint(256),
    DynSolType::Uint(256),
    DynSolValue::Uint(a, _),
    DynSolValue::Uint(b, _),
    a == b
);
assert_eq_cheatcode!(
    AssertEqInt,
    [0xfe, 0x74, 0xf0, 0x5b],
    DynSolType::Int(256),
    DynSolType::Int(256),
    DynSolValue::Int(a, _),
    DynSolValue::Int(b, _),
    a == b
);
assert_eq_cheatcode!(
    AssertEqAddress,
    [0x51, 0x53, 0x61, 0xf6],
    DynSolType::Address,
    DynSolType::Address,
    DynSolValue::Address(a),
    DynSolValue::Address(b),
    a == b
);
assert_eq_cheatcode!(
    AssertEqBytes32,
    [0x7c, 0x84, 0xc6, 0x9b],
    DynSolType::FixedBytes(32),
    DynSolType::FixedBytes(32),
    DynSolValue::FixedBytes(a, _),
    DynSolValue::FixedBytes(b, _),
    a == b
);
assert_eq_cheatcode!(
    AssertEqString,
    [0xf3, 0x20, 0xd9, 0x63],
    DynSolType::String,
    DynSolType::String,
    DynSolValue::String(a),
    DynSolValue::String(b),
    a == b
);
assert_eq_cheatcode!(
    AssertEqBytes,
    [0x97, 0x62, 0x46, 0x31],
    DynSolType::Bytes,
    DynSolType::Bytes,
    DynSolValue::Bytes(a),
    DynSolValue::Bytes(b),
    a == b
);

assert_not_eq_cheatcode!(
    AssertNotEqBool,
    [0x23, 0x6e, 0x4d, 0x66],
    DynSolType::Bool,
    DynSolType::Bool,
    DynSolValue::Bool(a),
    DynSolValue::Bool(b),
    a == b
);
assert_not_eq_cheatcode!(
    AssertNotEqUint,
    [0xb7, 0x90, 0x93, 0x20],
    DynSolType::Uint(256),
    DynSolType::Uint(256),
    DynSolValue::Uint(a, _),
    DynSolValue::Uint(b, _),
    a == b
);
assert_not_eq_cheatcode!(
    AssertNotEqInt,
    [0xf4, 0xc0, 0x04, 0xe3],
    DynSolType::Int(256),
    DynSolType::Int(256),
    DynSolValue::Int(a, _),
    DynSolValue::Int(b, _),
    a == b
);
assert_not_eq_cheatcode!(
    AssertNotEqAddress,
    [0xb1, 0x2e, 0x16, 0x94],
    DynSolType::Address,
    DynSolType::Address,
    DynSolValue::Address(a),
    DynSolValue::Address(b),
    a == b
);
assert_not_eq_cheatcode!(
    AssertNotEqBytes32,
    [0x89, 0x8e, 0x83, 0xfc],
    DynSolType::FixedBytes(32),
    DynSolType::FixedBytes(32),
    DynSolValue::FixedBytes(a, _),
    DynSolValue::FixedBytes(b, _),
    a == b
);
assert_not_eq_cheatcode!(
    AssertNotEqString,
    [0x6a, 0x82, 0x37, 0xb3],
    DynSolType::String,
    DynSolType::String,
    DynSolValue::String(a),
    DynSolValue::String(b),
    a == b
);
assert_not_eq_cheatcode!(
    AssertNotEqBytes,
    [0x3c, 0xf7, 0x8e, 0x28],
    DynSolType::Bytes,
    DynSolType::Bytes,
    DynSolValue::Bytes(a),
    DynSolValue::Bytes(b),
    a == b
);

pub struct AssertTrue;
impl Cheatcode for AssertTrue {
    type Args = bool;
    const SELECTOR: [u8; 4] = [0x0c, 0x9f, 0xd5, 0x81];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_single_bool(input)
    }
    fn effects(b: Self::Args) -> Vec<CheatcodeEffect> {
        if b {
            vec![]
        } else {
            vec![CheatcodeEffect::Panic]
        }
    }
}

pub struct AssertFalse;
impl Cheatcode for AssertFalse {
    type Args = bool;
    const SELECTOR: [u8; 4] = [0xa5, 0x98, 0x28, 0x85];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_single_bool(input)
    }
    fn effects(b: Self::Args) -> Vec<CheatcodeEffect> {
        if !b {
            vec![]
        } else {
            vec![CheatcodeEffect::Panic]
        }
    }
}

fn cmp_uint_outcome(input: &Bytes, expect_less: bool, expect_equal: bool) -> Vec<CheatcodeEffect> {
    let Some((a, b)) = decode_pair(input, DynSolType::Uint(256), DynSolType::Uint(256)) else {
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
    if ok {
        vec![]
    } else {
        vec![CheatcodeEffect::Panic]
    }
}

fn cmp_int_outcome(input: &Bytes, expect_less: bool, expect_equal: bool) -> Vec<CheatcodeEffect> {
    let Some((a, b)) = decode_pair(input, DynSolType::Int(256), DynSolType::Int(256)) else {
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
    if ok {
        vec![]
    } else {
        vec![CheatcodeEffect::Panic]
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
    AssertLtUint,
    [0xb1, 0x2f, 0xc0, 0x05],
    cmp_uint_outcome,
    true,
    false
);
cmp_cheatcode!(
    AssertLtInt,
    [0x3e, 0x91, 0x40, 0x80],
    cmp_int_outcome,
    true,
    false
);
cmp_cheatcode!(
    AssertLeUint,
    [0x84, 0x66, 0xf4, 0x15],
    cmp_uint_outcome,
    true,
    true
);
cmp_cheatcode!(
    AssertLeInt,
    [0x95, 0xfd, 0x15, 0x4e],
    cmp_int_outcome,
    true,
    true
);
cmp_cheatcode!(
    AssertGtUint,
    [0xdb, 0x07, 0xfc, 0xd2],
    cmp_uint_outcome,
    false,
    false
);
cmp_cheatcode!(
    AssertGtInt,
    [0x5a, 0x36, 0x2d, 0x45],
    cmp_int_outcome,
    false,
    false
);
cmp_cheatcode!(
    AssertGeUint,
    [0xa8, 0xd4, 0xd1, 0xd9],
    cmp_uint_outcome,
    false,
    true
);
cmp_cheatcode!(
    AssertGeInt,
    [0x0a, 0x30, 0xb7, 0x71],
    cmp_int_outcome,
    false,
    true
);

#[cfg(test)]
mod tests {
    use std::path::Path;

    use alloy_primitives::U256;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    fn call_data(selector: [u8; 4], encoded: Vec<u8>) -> Bytes {
        let mut data = selector.to_vec();
        data.extend(encoded);
        Bytes::from(data)
    }

    #[test]
    fn assert_true_passes() {
        let encoded = DynSolValue::Bool(true).abi_encode();
        let args = AssertTrue::decode(&call_data(AssertTrue::SELECTOR, encoded)).unwrap();
        let effects = AssertTrue::effects(args);
        assert!(effects.is_empty());
    }

    #[test]
    fn assert_true_fails() {
        let encoded = DynSolValue::Bool(false).abi_encode();
        let args = AssertTrue::decode(&call_data(AssertTrue::SELECTOR, encoded)).unwrap();
        let effects = AssertTrue::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::Panic]);
    }

    #[test]
    fn assert_eq_uint_passes() {
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Uint(U256::from(42u64), 256),
            DynSolValue::Uint(U256::from(42u64), 256),
        ])
        .abi_encode();
        let args = AssertEqUint::decode(&call_data(AssertEqUint::SELECTOR, encoded)).unwrap();
        let effects = AssertEqUint::effects(args);
        assert!(effects.is_empty());
    }

    #[test]
    fn assert_lt_uint_fails_when_greater() {
        let encoded = DynSolValue::Tuple(vec![
            DynSolValue::Uint(U256::from(100u64), 256),
            DynSolValue::Uint(U256::from(42u64), 256),
        ])
        .abi_encode();
        let args = AssertLtUint::decode(&call_data(AssertLtUint::SELECTOR, encoded)).unwrap();
        let effects = AssertLtUint::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::Panic]);
    }

    #[test]
    fn assert_false_passes() {
        let encoded = DynSolValue::Bool(false).abi_encode();
        let args = AssertFalse::decode(&call_data(AssertFalse::SELECTOR, encoded)).unwrap();
        let effects = AssertFalse::effects(args);
        assert!(effects.is_empty());
    }

    #[test]
    fn assert_false_fails() {
        let encoded = DynSolValue::Bool(true).abi_encode();
        let args = AssertFalse::decode(&call_data(AssertFalse::SELECTOR, encoded)).unwrap();
        let effects = AssertFalse::effects(args);
        assert_eq!(effects, vec![CheatcodeEffect::Panic]);
    }

    #[test]
    fn assert_panic_encoding_matches_solidity() {
        let result = crate::chain::cheatcodes::panic_outcome();
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
