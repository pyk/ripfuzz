//! A single call in a fuzzing sequence.

use alloy_dyn_abi::{DynSolType, DynSolValue, Specifier};
use alloy_json_abi::Function;
use alloy_primitives::{keccak256, Address, FixedBytes, Selector, U256};
use revm::primitives::Bytes;
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
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("sig", &self.function.signature())?;
        map.serialize_entry("args", &hex::encode(self.args.abi_encode_params()))?;
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
            args: String,
            #[serde(default)]
            value: U256,
            #[serde(default = "default_deployer")]
            caller: Address,
        }

        let helper = CallHelper::deserialize(deserializer)?;
        let function = Function::parse(&helper.sig).map_err(serde::de::Error::custom)?;
        let raw_args = alloy_primitives::hex::decode(helper.args.trim_start_matches("0x"))
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
        let args = tuple
            .abi_decode_params(&raw_args)
            .map_err(|e| serde::de::Error::custom(e.to_string()))?;

        Ok(Self {
            function,
            args,
            value: helper.value,
            caller: helper.caller,
        })
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

    /// Create an owned copy of this call without using `Clone::clone`.
    pub fn replicate(&self) -> Self {
        Self {
            function: self.function.clone(),
            args: self.args.clone(),
            value: self.value,
            caller: self.caller,
        }
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
