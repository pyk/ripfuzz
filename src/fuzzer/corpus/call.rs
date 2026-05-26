//! A single call in a fuzzing sequence.

use alloy_dyn_abi::{DynSolType, DynSolValue, Specifier};
use alloy_json_abi::Function;
use alloy_primitives::{Address, FixedBytes, I256, Selector, U256, keccak256};
use revm::primitives::Bytes;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};

/// A single call in a sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// Target function definition (name, input types, state mutability).
    pub function: Function,
    /// Concrete argument values. Always a [`DynSolValue::Tuple`] whose
    /// elements match `function.inputs` in order.
    pub args: DynSolValue,
    /// Wei value sent with this call.
    pub value: U256,
    /// Account address that sends this call.
    pub caller: Address,
}

impl Default for Call {
    fn default() -> Self {
        Self {
            function: Function {
                name: String::new(),
                inputs: vec![],
                outputs: vec![],
                state_mutability: alloy_json_abi::StateMutability::NonPayable,
            },
            args: DynSolValue::Tuple(vec![]),
            value: U256::ZERO,
            caller: crate::evm::chain::DEFAULT_DEPLOYER,
        }
    }
}

impl Serialize for Call {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("sig", &self.function.signature())?;
        map.serialize_entry("args", &dyn_value_to_json(&self.args))?;
        map.serialize_entry("value", &self.value)?;
        map.serialize_entry("caller", &self.caller)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Call {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct CallHelper {
            sig: String,
            args: serde_json::Value,
            #[serde(default)]
            value: U256,
            #[serde(default = "default_deployer")]
            caller: Address,
        }

        let helper = CallHelper::deserialize(deserializer)?;
        let function = Function::parse(&helper.sig).map_err(serde::de::Error::custom)?;

        let types: Vec<DynSolType> = function
            .inputs
            .iter()
            .map(|p| {
                p.resolve()
                    .map_err(|e: alloy_dyn_abi::Error| serde::de::Error::custom(format!("{}", e)))
            })
            .collect::<Result<Vec<DynSolType>, D::Error>>()?;
        let tuple = DynSolType::Tuple(types);
        let args = json_to_dyn_value(&helper.args, &tuple).map_err(serde::de::Error::custom)?;

        Ok(Self {
            function,
            args,
            value: helper.value,
            caller: helper.caller,
        })
    }
}

/// Convert a [`DynSolValue`] into a human-friendly JSON representation.
fn dyn_value_to_json(value: &DynSolValue) -> serde_json::Value {
    match value {
        DynSolValue::Bool(b) => serde_json::Value::Bool(*b),
        DynSolValue::Uint(u, _) => serde_json::Value::String(format!("{}", *u)),
        DynSolValue::Int(i, _) => serde_json::Value::String(format!("{}", *i)),
        DynSolValue::Address(a) => serde_json::Value::String(format!("{:?}", a)),
        DynSolValue::FixedBytes(b, sz) => {
            serde_json::Value::String(format!("0x{}", hex::encode(&b.as_slice()[..*sz])))
        }
        DynSolValue::Bytes(b) => serde_json::Value::String(format!("0x{}", hex::encode(b))),
        DynSolValue::String(s) => serde_json::Value::String(s.clone()),
        DynSolValue::Function(f) => serde_json::Value::String(format!("{:?}", f)),
        DynSolValue::Array(arr) | DynSolValue::FixedArray(arr) => {
            serde_json::Value::Array(arr.iter().map(dyn_value_to_json).collect())
        }
        DynSolValue::Tuple(arr) => {
            serde_json::Value::Array(arr.iter().map(dyn_value_to_json).collect())
        }
    }
}

/// Parse a human-friendly JSON value back into a [`DynSolValue`] using the
/// expected Solidity type.
fn json_to_dyn_value(json: &serde_json::Value, ty: &DynSolType) -> Result<DynSolValue, String> {
    match (ty, json) {
        (DynSolType::Bool, serde_json::Value::Bool(b)) => Ok(DynSolValue::Bool(*b)),
        (DynSolType::Uint(sz), serde_json::Value::String(s)) => {
            let u = s.parse::<U256>().map_err(|e| format!("{}", e))?;
            Ok(DynSolValue::Uint(u, *sz))
        }
        (DynSolType::Uint(sz), serde_json::Value::Number(n)) => {
            let u = format!("{}", n)
                .parse::<U256>()
                .map_err(|e| format!("{}", e))?;
            Ok(DynSolValue::Uint(u, *sz))
        }
        (DynSolType::Int(sz), serde_json::Value::String(s)) => {
            let i = s.parse::<I256>().map_err(|e| format!("{}", e))?;
            Ok(DynSolValue::Int(i, *sz))
        }
        (DynSolType::Int(sz), serde_json::Value::Number(n)) => {
            let i = format!("{}", n)
                .parse::<I256>()
                .map_err(|e| format!("{}", e))?;
            Ok(DynSolValue::Int(i, *sz))
        }
        (DynSolType::Address, serde_json::Value::String(s)) => {
            let a = s.parse::<Address>().map_err(|e| format!("{}", e))?;
            Ok(DynSolValue::Address(a))
        }
        (DynSolType::FixedBytes(sz), serde_json::Value::String(s)) => {
            let bytes = alloy_primitives::hex::decode(s.trim_start_matches("0x"))
                .map_err(|e| format!("{}", e))?;
            let mut word = [0u8; 32];
            word[..bytes.len().min(32)].copy_from_slice(&bytes);
            Ok(DynSolValue::FixedBytes(FixedBytes::from(word), *sz))
        }
        (DynSolType::Bytes, serde_json::Value::String(s)) => {
            let bytes = alloy_primitives::hex::decode(s.trim_start_matches("0x"))
                .map_err(|e| format!("{}", e))?;
            Ok(DynSolValue::Bytes(bytes))
        }
        (DynSolType::String, serde_json::Value::String(s)) => Ok(DynSolValue::String(s.clone())),
        (DynSolType::Function, serde_json::Value::String(s)) => {
            let f = s
                .parse::<alloy_primitives::Function>()
                .map_err(|e| format!("{}", e))?;
            Ok(DynSolValue::Function(f))
        }
        (DynSolType::Array(inner), serde_json::Value::Array(arr)) => {
            let values = arr
                .iter()
                .map(|v| json_to_dyn_value(v, inner))
                .collect::<Result<Vec<DynSolValue>, String>>()?;
            Ok(DynSolValue::Array(values))
        }
        (DynSolType::FixedArray(inner, len), serde_json::Value::Array(arr)) => {
            if arr.len() != *len {
                return Err(format!("expected {} elements, got {}", len, arr.len()));
            }
            let values = arr
                .iter()
                .map(|v| json_to_dyn_value(v, inner))
                .collect::<Result<Vec<DynSolValue>, String>>()?;
            Ok(DynSolValue::FixedArray(values))
        }
        (DynSolType::Tuple(types), serde_json::Value::Array(arr)) => {
            if arr.len() != types.len() {
                return Err(format!(
                    "expected {} elements, got {}",
                    types.len(),
                    arr.len()
                ));
            }
            let values = arr
                .iter()
                .zip(types.iter())
                .map(|(v, t)| json_to_dyn_value(v, t))
                .collect::<Result<Vec<DynSolValue>, String>>()?;
            Ok(DynSolValue::Tuple(values))
        }
        _ => Err(format!("type mismatch: expected {:?}, got {:?}", ty, json)),
    }
}

impl Call {
    /// 4-byte function selector.
    pub fn selector(&self) -> Selector {
        self.function.selector()
    }

    /// Encode this call as EVM calldata (selector + ABI-encoded args).
    pub fn calldata(&self) -> Bytes {
        let args = self.args.abi_encode_params();
        let mut buf = Vec::with_capacity(4 + args.len());
        buf.extend_from_slice(self.function.selector().as_slice());
        buf.extend_from_slice(&args);
        Bytes::from(buf)
    }

    /// Deterministic Keccak256 hash of the fields that affect EVM execution.
    ///
    /// This includes `caller`, `value`, and the calldata, because all three
    /// can change the resulting state transition.
    pub fn content_hash(&self) -> [u8; 32] {
        let mut buf = Vec::new();
        buf.extend_from_slice(self.caller.as_slice());
        buf.extend_from_slice(&self.value.to_be_bytes::<32>());
        buf.extend_from_slice(&self.calldata());
        keccak256(&buf).into()
    }

    /// Convert this call into an EVM [`Transaction`](crate::evm::chain::Transaction)
    /// directed at `target`.
    pub fn into_transaction(&self, target: Address) -> crate::evm::chain::Transaction {
        crate::evm::chain::Transaction::new(target)
            .caller(self.caller)
            .calldata(self.calldata())
            .value(self.value)
    }
}

fn default_deployer() -> Address {
    crate::evm::chain::DEFAULT_DEPLOYER
}

/// Generate a default [`DynSolValue`] for the given type.
///
/// Used to create placeholder arguments when generating random sequences.
pub fn default_dyn_value(ty: &DynSolType) -> DynSolValue {
    match ty {
        DynSolType::Bool => DynSolValue::Bool(false),
        DynSolType::Uint(sz) => DynSolValue::Uint(alloy_primitives::U256::ZERO, *sz),
        DynSolType::Int(sz) => DynSolValue::Int(alloy_primitives::I256::ZERO, *sz),
        DynSolType::FixedBytes(sz) => DynSolValue::FixedBytes(FixedBytes::default(), *sz),
        DynSolType::Address => DynSolValue::Address(alloy_primitives::Address::ZERO),
        DynSolType::Function => DynSolValue::Function(alloy_primitives::Function::ZERO),
        DynSolType::Bytes => DynSolValue::Bytes(vec![]),
        DynSolType::String => DynSolValue::String(String::new()),
        DynSolType::Array(_) => DynSolValue::Array(vec![]),
        DynSolType::FixedArray(inner, len) => {
            DynSolValue::FixedArray((0..*len).map(|_| default_dyn_value(inner)).collect())
        }
        DynSolType::Tuple(types) => {
            DynSolValue::Tuple(types.iter().map(default_dyn_value).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;
    use alloy_primitives::{Address, U256};

    use super::*;

    fn call_transfer() -> Call {
        Call {
            function: Function::parse("transfer(address,uint256)").unwrap(),
            args: DynSolValue::Tuple(vec![
                DynSolValue::Address(Address::from([0xab; 20])),
                DynSolValue::Uint(U256::from(42), 256),
            ]),
            value: U256::ZERO,
            caller: Address::from([0xcd; 20]),
        }
    }

    fn call_empty() -> Call {
        Call {
            function: Function::parse("foo()").unwrap(),
            args: DynSolValue::Tuple(vec![]),
            value: U256::ZERO,
            caller: crate::evm::chain::DEFAULT_DEPLOYER,
        }
    }

    fn call_complex() -> Call {
        Call {
            function: Function::parse("set(bytes32,uint256[],(bool,address))").unwrap(),
            args: DynSolValue::Tuple(vec![
                DynSolValue::FixedBytes(FixedBytes::from([0x12; 32]), 32),
                DynSolValue::Array(vec![
                    DynSolValue::Uint(U256::from(1), 256),
                    DynSolValue::Uint(U256::from(2), 256),
                ]),
                DynSolValue::Tuple(vec![
                    DynSolValue::Bool(true),
                    DynSolValue::Address(Address::from([0xef; 20])),
                ]),
            ]),
            value: U256::from(1_000),
            caller: Address::from([0x11; 20]),
        }
    }

    #[test]
    fn serde_round_trip_preserves_calldata() {
        let original = call_transfer();
        let json = serde_json::to_string(&original).unwrap();
        let roundtrip: Call = serde_json::from_str(&json).unwrap();
        assert_eq!(original.calldata(), roundtrip.calldata());
    }

    #[test]
    fn serde_round_trip_preserves_content_hash() {
        let original = call_transfer();
        let json = serde_json::to_string(&original).unwrap();
        let roundtrip: Call = serde_json::from_str(&json).unwrap();
        assert_eq!(original.content_hash(), roundtrip.content_hash());
    }

    #[test]
    fn serde_defaults_for_missing_value_and_caller() {
        let json = r#"{"sig":"foo()","args":[]}"#;
        let call: Call = serde_json::from_str(json).unwrap();
        assert_eq!(call.value, U256::ZERO);
        assert_eq!(call.caller, crate::evm::chain::DEFAULT_DEPLOYER);
    }

    #[test]
    fn serde_round_trip_empty_args() {
        let original = call_empty();
        let json = serde_json::to_string(&original).unwrap();
        let roundtrip: Call = serde_json::from_str(&json).unwrap();
        assert_eq!(original.calldata(), roundtrip.calldata());
    }

    #[test]
    fn serde_round_trip_complex_types() {
        let original = call_complex();
        let json = serde_json::to_string(&original).unwrap();
        let roundtrip: Call = serde_json::from_str(&json).unwrap();
        assert_eq!(original.content_hash(), roundtrip.content_hash());
        assert_eq!(original.value, roundtrip.value);
        assert_eq!(original.caller, roundtrip.caller);
    }

    #[test]
    fn fixture_file_round_trip() {
        let dir = "fixtures/corpus";
        let _ = fs::create_dir_all(dir);

        let path = format!("{}/call_sample_transfer.json", dir);
        let original = call_transfer();
        let json = serde_json::to_string_pretty(&original).unwrap();
        fs::write(&path, json).unwrap();

        let read = fs::read_to_string(&path).unwrap();
        let roundtrip: Call = serde_json::from_str(&read).unwrap();
        assert_eq!(original.content_hash(), roundtrip.content_hash());
        assert_eq!(original.calldata(), roundtrip.calldata());
    }

    #[test]
    fn fixture_file_complex_round_trip() {
        let dir = "fixtures/corpus";
        let _ = fs::create_dir_all(dir);

        let path = format!("{}/call_sample_complex.json", dir);
        let original = call_complex();
        let json = serde_json::to_string_pretty(&original).unwrap();
        fs::write(&path, json).unwrap();

        let read = fs::read_to_string(&path).unwrap();
        let roundtrip: Call = serde_json::from_str(&read).unwrap();
        assert_eq!(original.content_hash(), roundtrip.content_hash());
    }
}
