//! `toString` cheatcodes — pure type-to-string conversion.

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

fn encode_string(s: &str) -> Vec<u8> {
    DynSolValue::String(s.into()).abi_encode()
}

// ---------------------------------------------------------------------------
// toString(address)
// ---------------------------------------------------------------------------
pub struct ToStringAddress;

impl Cheatcode for ToStringAddress {
    type Args = DynSolValue;
    const SELECTOR: [u8; 4] = [0x56, 0xca, 0x62, 0x3e];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_single(input, DynSolType::Address)
    }

    fn effects(val: Self::Args) -> Vec<CheatcodeEffect> {
        let DynSolValue::Address(addr) = val else {
            return vec![];
        };
        vec![CheatcodeEffect::ReturnBytes(encode_string(&format!(
            "{}",
            addr
        )))]
    }
}

// ---------------------------------------------------------------------------
// toString(bool)
// ---------------------------------------------------------------------------
pub struct ToStringBool;

impl Cheatcode for ToStringBool {
    type Args = DynSolValue;
    const SELECTOR: [u8; 4] = [0x71, 0xdc, 0xe7, 0xda];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_single(input, DynSolType::Bool)
    }

    fn effects(val: Self::Args) -> Vec<CheatcodeEffect> {
        let DynSolValue::Bool(b) = val else {
            return vec![];
        };
        vec![CheatcodeEffect::ReturnBytes(encode_string(&format!(
            "{}",
            b
        )))]
    }
}

// ---------------------------------------------------------------------------
// toString(uint256)
// ---------------------------------------------------------------------------
pub struct ToStringUint;

impl Cheatcode for ToStringUint {
    type Args = DynSolValue;
    const SELECTOR: [u8; 4] = [0x69, 0x00, 0xa3, 0xae];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_single(input, DynSolType::Uint(256))
    }

    fn effects(val: Self::Args) -> Vec<CheatcodeEffect> {
        let DynSolValue::Uint(u, _) = val else {
            return vec![];
        };
        vec![CheatcodeEffect::ReturnBytes(encode_string(&format!(
            "{}",
            u
        )))]
    }
}

// ---------------------------------------------------------------------------
// toString(int256)
// ---------------------------------------------------------------------------
pub struct ToStringInt;

impl Cheatcode for ToStringInt {
    type Args = DynSolValue;
    const SELECTOR: [u8; 4] = [0xa3, 0x22, 0xc4, 0x0e];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_single(input, DynSolType::Int(256))
    }

    fn effects(val: Self::Args) -> Vec<CheatcodeEffect> {
        let DynSolValue::Int(i, _) = val else {
            return vec![];
        };
        vec![CheatcodeEffect::ReturnBytes(encode_string(&format!(
            "{}",
            i
        )))]
    }
}

// ---------------------------------------------------------------------------
// toString(bytes32)
// ---------------------------------------------------------------------------
pub struct ToStringBytes32;

impl Cheatcode for ToStringBytes32 {
    type Args = DynSolValue;
    const SELECTOR: [u8; 4] = [0xb1, 0x1a, 0x19, 0xe8];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_single(input, DynSolType::FixedBytes(32))
    }

    fn effects(val: Self::Args) -> Vec<CheatcodeEffect> {
        let DynSolValue::FixedBytes(b, _) = val else {
            return vec![];
        };
        vec![CheatcodeEffect::ReturnBytes(encode_string(&format!(
            "0x{}",
            hex::encode(b)
        )))]
    }
}

// ---------------------------------------------------------------------------
// toString(bytes)
// ---------------------------------------------------------------------------
pub struct ToStringBytes;

impl Cheatcode for ToStringBytes {
    type Args = DynSolValue;
    const SELECTOR: [u8; 4] = [0x71, 0xaa, 0xd1, 0x0d];

    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_single(input, DynSolType::Bytes)
    }

    fn effects(val: Self::Args) -> Vec<CheatcodeEffect> {
        let DynSolValue::Bytes(b) = val else {
            return vec![];
        };
        vec![CheatcodeEffect::ReturnBytes(encode_string(&format!(
            "0x{}",
            hex::encode(b)
        )))]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serial_test::serial;

    use alloy_primitives::{Address, I256, U256};

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
    fn to_string_address_decode_and_effects() {
        let addr =
            Address::from_slice(&hex::decode("263Af513A0435EBC9D5C362Cf76252F87173F8f1").unwrap());
        let encoded = DynSolValue::Address(addr).abi_encode();
        let args = ToStringAddress::decode(&call_data(ToStringAddress::SELECTOR, encoded)).unwrap();
        let effects = ToStringAddress::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String("0x263Af513A0435EBC9D5C362Cf76252F87173F8f1".into())
        );
    }

    #[test]
    fn to_string_bool_true() {
        let encoded = DynSolValue::Bool(true).abi_encode();
        let args = ToStringBool::decode(&call_data(ToStringBool::SELECTOR, encoded)).unwrap();
        let effects = ToStringBool::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String("true".into()));
    }

    #[test]
    fn to_string_bool_false() {
        let encoded = DynSolValue::Bool(false).abi_encode();
        let args = ToStringBool::decode(&call_data(ToStringBool::SELECTOR, encoded)).unwrap();
        let effects = ToStringBool::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String("false".into()));
    }

    #[test]
    fn to_string_uint_zero() {
        let encoded = DynSolValue::Uint(U256::ZERO, 256).abi_encode();
        let args = ToStringUint::decode(&call_data(ToStringUint::SELECTOR, encoded)).unwrap();
        let effects = ToStringUint::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String("0".into()));
    }

    #[test]
    fn to_string_uint_max() {
        let encoded = DynSolValue::Uint(U256::MAX, 256).abi_encode();
        let args = ToStringUint::decode(&call_data(ToStringUint::SELECTOR, encoded)).unwrap();
        let effects = ToStringUint::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String(U256::MAX.to_string().into()));
    }

    #[test]
    fn to_string_int_zero() {
        let encoded = DynSolValue::Int(I256::ZERO, 256).abi_encode();
        let args = ToStringInt::decode(&call_data(ToStringInt::SELECTOR, encoded)).unwrap();
        let effects = ToStringInt::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String("0".into()));
    }

    #[test]
    fn to_string_int_min() {
        let encoded = DynSolValue::Int(I256::MIN, 256).abi_encode();
        let args = ToStringInt::decode(&call_data(ToStringInt::SELECTOR, encoded)).unwrap();
        let effects = ToStringInt::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(
                "-57896044618658097711785492504343953926634992332820282019728792003956564819968"
                    .into()
            )
        );
    }

    #[test]
    fn to_string_int_positive() {
        let encoded = DynSolValue::Int(I256::try_from(42i64).unwrap(), 256).abi_encode();
        let args = ToStringInt::decode(&call_data(ToStringInt::SELECTOR, encoded)).unwrap();
        let effects = ToStringInt::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String("42".into()));
    }

    #[test]
    fn to_string_bytes32_zero() {
        let encoded = DynSolValue::FixedBytes([0u8; 32].into(), 32).abi_encode();
        let args = ToStringBytes32::decode(&call_data(ToStringBytes32::SELECTOR, encoded)).unwrap();
        let effects = ToStringBytes32::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(
                "0x0000000000000000000000000000000000000000000000000000000000000000".into()
            )
        );
    }

    #[test]
    fn to_string_bytes32_arbitrary() {
        let mut arr = [0u8; 32];
        arr[0..4].copy_from_slice(&hex::decode("deadbeef").unwrap());
        let encoded = DynSolValue::FixedBytes(arr.into(), 32).abi_encode();
        let args = ToStringBytes32::decode(&call_data(ToStringBytes32::SELECTOR, encoded)).unwrap();
        let effects = ToStringBytes32::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String(
                "0xdeadbeef00000000000000000000000000000000000000000000000000000000".into()
            )
        );
    }

    #[test]
    fn to_string_bytes_empty() {
        let encoded = DynSolValue::Bytes(vec![]).abi_encode();
        let args = ToStringBytes::decode(&call_data(ToStringBytes::SELECTOR, encoded)).unwrap();
        let effects = ToStringBytes::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String("0x".into()));
    }

    #[test]
    fn to_string_bytes_nonempty() {
        let encoded = DynSolValue::Bytes(vec![0x01, 0xab]).abi_encode();
        let args = ToStringBytes::decode(&call_data(ToStringBytes::SELECTOR, encoded)).unwrap();
        let effects = ToStringBytes::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String("0x01ab".into()));
    }

    #[test]
    fn to_string_zero_address() {
        let addr = Address::ZERO;
        let encoded = DynSolValue::Address(addr).abi_encode();
        let args = ToStringAddress::decode(&call_data(ToStringAddress::SELECTOR, encoded)).unwrap();
        let effects = ToStringAddress::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(
            decoded,
            DynSolValue::String("0x0000000000000000000000000000000000000000".into())
        );
    }

    #[test]
    fn decode_nonempty_bytes_from_solidity() {
        let input = hex::decode("71aad10d0000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000201ab000000000000000000000000000000000000000000000000000000000000").unwrap();
        let args = ToStringBytes::decode(&Bytes::from(input)).unwrap();
        let DynSolValue::Bytes(ref b) = args else {
            panic!("expected Bytes");
        };
        assert_eq!(*b, vec![0x01, 0xab]);
        let effects = ToStringBytes::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String("0x01ab".into()));
    }

    #[test]
    fn to_string_uint_already_works() {
        let encoded = DynSolValue::Uint(U256::from(123u64), 256).abi_encode();
        let args = ToStringUint::decode(&call_data(ToStringUint::SELECTOR, encoded)).unwrap();
        let effects = ToStringUint::effects(args);
        let CheatcodeEffect::ReturnBytes(out) = &effects[0] else {
            panic!("expected ReturnBytes");
        };
        let decoded = DynSolType::String.abi_decode_params(out).unwrap();
        assert_eq!(decoded, DynSolValue::String("123".into()));
    }

    #[test]
    #[serial]
    fn cheatcode_to_string_setup_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeToString.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let output = chain.execute(&[]).unwrap();
        assert!(output.all_ok, "setup should succeed");
        assert!(
            output.crash.is_none(),
            "no invariant should crash after setup"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_to_string_edge_cases_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeToString.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let call_zero_uint: [u8; 4] = [0xe4, 0x1b, 0x29, 0x67]; // action_toStringZeroUint()
        let call_max_uint: [u8; 4] = [0x13, 0x78, 0x98, 0xec]; // action_toStringMaxUint()
        let call_min_int: [u8; 4] = [0x0c, 0x93, 0x3b, 0xdf]; // action_toStringMinInt()
        let call_zero_addr: [u8; 4] = [0xe1, 0xb4, 0x1c, 0x11]; // action_toStringZeroAddress()
        let call_false: [u8; 4] = [0x16, 0x5b, 0x3b, 0xdd]; // action_toStringFalse()
        let call_empty_bytes: [u8; 4] = [0x68, 0xc7, 0x0e, 0x7a]; // action_toStringEmptyBytes()
        let call_empty_b32: [u8; 4] = [0x8a, 0x1c, 0xe3, 0xff]; // action_toStringEmptyBytes32()

        let calls = vec![
            Call {
                selector: call_zero_uint,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_max_uint,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_min_int,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_zero_addr,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_false,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_empty_bytes,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_empty_b32,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "edge-case actions should succeed");
        assert!(
            output.crash.is_none(),
            "no invariant should crash after edge-case actions"
        );
    }

    #[test]
    #[serial]
    fn cheatcode_to_string_side_effect_isolation_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeToString.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let call_warp: [u8; 4] = [0xe1, 0xea, 0xa3, 0x64]; // action_toStringThenWarp()
        let call_roll: [u8; 4] = [0xb3, 0xbb, 0x26, 0x2d]; // action_toStringThenRoll()
        let call_two: [u8; 4] = [0x3c, 0x30, 0xdf, 0xe4]; // action_twoToStringCalls()

        let calls = vec![
            Call {
                selector: call_warp,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_roll,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_two,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "side-effect actions should succeed");
    }

    #[test]
    #[serial]
    fn cheatcode_to_string_round_trip_integration() {
        let artifact = contract::ContractBuilder::for_project(Path::new("fixtures/cheatcodes"))
            .with_target_path(Path::new("test/CheatcodeToString.sol"))
            .build()
            .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let call_rt_uint: [u8; 4] = [0x76, 0xff, 0xa8, 0x77]; // action_roundTripUint(uint256)
        let call_rt_int: [u8; 4] = [0x71, 0xd9, 0xdb, 0x42]; // action_roundTripInt(int256)
        let call_rt_addr: [u8; 4] = [0xcd, 0xbe, 0x54, 0xbe]; // action_roundTripAddress(address)
        let call_rt_bool: [u8; 4] = [0x6e, 0x13, 0x5a, 0x51]; // action_roundTripBool(bool)
        let call_rt_b32: [u8; 4] = [0x52, 0x3f, 0x03, 0xa6]; // action_roundTripBytes32(bytes32)
        let call_rt_bytes: [u8; 4] = [0x95, 0x6a, 0xa6, 0x9a]; // action_roundTripBytes(bytes)

        let mut args_uint = vec![0u8; 32];
        args_uint[24..32].copy_from_slice(&12345u64.to_be_bytes());

        let mut args_int = vec![0u8; 32];
        // negative value: two's complement of -123
        let neg_123: [u8; 32] = {
            let mut b = [0u8; 32];
            b[31] = 123;
            let mut carry = true;
            for i in (0..32).rev() {
                b[i] = !b[i];
                if carry {
                    let (sum, c) = b[i].overflowing_add(1);
                    b[i] = sum;
                    carry = c;
                }
            }
            b
        };
        args_int.copy_from_slice(&neg_123);

        let mut args_addr = vec![0u8; 32];
        args_addr[12..32]
            .copy_from_slice(&hex::decode("7109709ECfa91a80626fF3989D68f67F5b1DD12D").unwrap());

        let mut args_bool = vec![0u8; 32];
        args_bool[31] = 1;

        let mut args_b32 = vec![0u8; 32];
        args_b32[0..32].copy_from_slice(
            &hex::decode("deadbeef00000000000000000000000000000000000000000000000000000000")
                .unwrap(),
        );

        let mut args_bytes = vec![0u8; 32];
        // bytes calldata: offset 0x20, length 2, then data 0x01ab padded (left-aligned)
        args_bytes[0..32].copy_from_slice(&{
            let mut b = [0u8; 32];
            b[31] = 0x20; // offset
            b
        });
        args_bytes.extend_from_slice(&{
            let mut b = [0u8; 32];
            b[31] = 2; // length
            b
        });
        args_bytes.extend_from_slice(&{
            let mut b = [0u8; 32];
            b[0] = 0x01;
            b[1] = 0xab;
            b
        });

        let calls = vec![
            Call {
                selector: call_rt_uint,
                args: args_uint,
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_rt_int,
                args: args_int,
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_rt_addr,
                args: args_addr,
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_rt_bool,
                args: args_bool,
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_rt_b32,
                args: args_b32,
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: call_rt_bytes,
                args: args_bytes,
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "round-trip actions should succeed");
        assert!(
            output.crash.is_none(),
            "no invariant should crash after round-trip actions"
        );
    }
}
