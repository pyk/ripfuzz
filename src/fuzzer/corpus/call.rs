//! A single call in a fuzzing sequence.

use alloy_dyn_abi::{DynSolType, DynSolValue, Specifier};
use alloy_json_abi::Function;
use alloy_primitives::{keccak256, FixedBytes, Selector};
use serde::{Deserialize, Serialize};

/// A single call in a sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// Target function definition (name, input types, state mutability).
    pub function: Function,
    /// Concrete argument values. Always a [`DynSolValue::Tuple`] whose
    /// elements match `function.inputs` in order.
    pub values: DynSolValue,
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
            values: DynSolValue::Tuple(vec![]),
        }
    }
}

impl Serialize for Call {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("sig", &self.function.signature())?;
        map.serialize_entry("args", &hex::encode(self.values.abi_encode_params()))?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Call {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct CallHelper {
            sig: String,
            args: String,
        }

        let helper = CallHelper::deserialize(deserializer)?;
        let function = Function::parse(&helper.sig).map_err(serde::de::Error::custom)?;
        let args = alloy_primitives::hex::decode(helper.args.trim_start_matches("0x"))
            .map_err(serde::de::Error::custom)?;

        let types: Vec<DynSolType> = function
            .inputs
            .iter()
            .map(|p| {
                p.resolve()
                    .map_err(|e: alloy_dyn_abi::Error| serde::de::Error::custom(e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let tuple = DynSolType::Tuple(types);
        let values = tuple
            .abi_decode_params(&args)
            .map_err(|e| serde::de::Error::custom(e.to_string()))?;

        Ok(Self { function, values })
    }
}

impl Call {
    /// 4-byte function selector.
    pub fn selector(&self) -> Selector {
        self.function.selector()
    }

    /// Encode this call as a flat byte vector (selector + ABI-encoded args).
    pub fn encode(&self) -> Vec<u8> {
        let args = self.values.abi_encode_params();
        let mut buf = Vec::with_capacity(4 + args.len());
        buf.extend_from_slice(self.function.selector().as_slice());
        buf.extend_from_slice(&args);
        buf
    }

    /// Deterministic Keccak256 hash of the fields that affect EVM execution.
    ///
    /// Human-readable metadata (`function`) is intentionally excluded
    /// because it is derived from the selector + values and does not change
    /// the state transition.
    pub fn content_hash(&self) -> [u8; 32] {
        let encoded = self.encode();
        keccak256(&encoded).into()
    }

    /// Create an owned copy of this call without using `Clone::clone`.
    pub fn replicate(&self) -> Self {
        Self {
            function: self.function.clone(),
            values: self.values.clone(),
        }
    }
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

fn default_true() -> bool {
    true
}

/// Metadata for a single call in an executed sequence.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CallMeta {
    /// Block number at execution time.
    pub block_number: u64,
    /// Block timestamp at execution time.
    pub block_timestamp: u64,
    /// Gas consumed by this individual call.
    #[serde(default)]
    pub gas_used: u64,
    /// Whether this call succeeded.
    #[serde(default = "default_true")]
    pub success: bool,
    /// If the call reverted or halted, the human-readable reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
