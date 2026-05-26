//! A single call in a fuzzing sequence.

use alloy_primitives::keccak256;
use serde::{Deserialize, Serialize};

/// A single call in a sequence.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Call {
    /// 4-byte function selector.
    pub selector: [u8; 4],
    /// ABI-encoded arguments (0 or more 32-byte words).
    pub args: Vec<u8>,
    /// Human-readable function name (empty when the selector is unknown).
    pub method_name: String,
    /// Full function signature, e.g. `transfer(address,uint256)`.
    pub method_signature: String,
    /// JSON-friendly representation of each ABI argument.
    pub input_values: Vec<serde_json::Value>,
}

impl Call {
    /// Total encoded size of this call (selector + args).
    pub fn encoded_size(&self) -> usize {
        4 + self.args.len()
    }

    /// Encode this call as a flat byte vector.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.encoded_size());
        buf.extend_from_slice(&self.selector);
        buf.extend_from_slice(&self.args);
        buf
    }

    /// Deterministic Keccak256 hash of the fields that affect EVM execution.
    ///
    /// Matches Medusa's approach: hashes the encoded calldata.
    /// Human-readable metadata (`method_name`, `method_signature`,
    /// `input_values`) is intentionally excluded because it is derived from
    /// `selector` + `args` and does not change the state transition.
    pub fn content_hash(&self) -> [u8; 32] {
        let mut buf = Vec::with_capacity(self.encoded_size());
        buf.extend_from_slice(&self.selector);
        buf.extend_from_slice(&self.args);
        keccak256(&buf).into()
    }

    /// Create an owned copy of this call without using `Clone::clone`.
    pub fn replicate(&self) -> Self {
        Self {
            selector: self.selector,
            args: self.args.to_vec(),
            method_name: self.method_name.clone(),
            method_signature: self.method_signature.clone(),
            input_values: self.input_values.clone(),
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
