//! Strongly typed JSON-RPC responses for EVM state fetches.

use alloy_primitives::{Address, B256, Bytes, U64, U256};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Typed block header (subset of `eth_getBlockByNumber` result).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Block {
    #[serde(rename = "number")]
    pub number: U64,
    #[serde(rename = "timestamp")]
    pub timestamp: U64,
    #[serde(rename = "miner")]
    pub coinbase: Address,
    #[serde(rename = "gasLimit")]
    pub gas_limit: U64,
    #[serde(rename = "baseFeePerGas", default)]
    pub basefee: U64,
    #[serde(rename = "mixHash", default)]
    pub prevrandao: Option<B256>,
    #[serde(rename = "difficulty", default)]
    pub difficulty: U256,
    #[serde(rename = "excessBlobGas", default)]
    pub excess_blob_gas: Option<U64>,
    #[serde(rename = "hash", default)]
    pub hash: Option<B256>,
}

/// Strongly typed JSON-RPC response for a single EVM state fetch.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Response {
    ChainId(u64),
    BlockByNumber(Block),
    Balance(U256),
    TransactionCount(u64),
    Code(Bytes),
    StorageAt(U256),
}

impl Response {
    /// Parse the `result` field of a JSON-RPC response into a typed value.
    pub fn parse(request: &super::request::Request, result: &serde_json::Value) -> Result<Self> {
        match request {
            super::request::Request::GetChainId { .. } => {
                let s = result.as_str().context("expected hex string for chainId")?;
                let v = U64::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16)
                    .context("invalid chainId hex")?;
                Ok(Self::ChainId(v.to()))
            }
            super::request::Request::GetBlockByNumber { .. } => {
                if result.is_null() {
                    bail!("block not found");
                }
                let block = Block::deserialize(result).context("invalid block response")?;
                Ok(Self::BlockByNumber(block))
            }
            super::request::Request::GetBalance { .. } => {
                let s = result.as_str().context("expected hex string for balance")?;
                let v: U256 = s.parse().context("invalid balance hex")?;
                Ok(Self::Balance(v))
            }
            super::request::Request::GetTransactionCount { .. } => {
                let s = result.as_str().context("expected hex string for nonce")?;
                let v = U64::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16)
                    .context("invalid nonce hex")?;
                Ok(Self::TransactionCount(v.to()))
            }
            super::request::Request::GetCode { .. } => {
                let s = result.as_str().context("expected hex string for code")?;
                let v: Bytes = s.parse().context("invalid code hex")?;
                Ok(Self::Code(v))
            }
            super::request::Request::GetStorageAt { .. } => {
                let s = result.as_str().context("expected hex string for storage")?;
                let v: U256 = s.parse().context("invalid storage hex")?;
                Ok(Self::StorageAt(v))
            }
        }
    }

    /// Serialize back to the raw JSON-RPC `result` shape for caching.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::ChainId(v) => serde_json::Value::String(format!("0x{v:x}")),
            Self::BlockByNumber(b) => serde_json::to_value(b).unwrap_or_default(),
            Self::Balance(v) => serde_json::Value::String(format!("0x{v:x}")),
            Self::TransactionCount(v) => serde_json::Value::String(format!("0x{v:x}")),
            Self::Code(v) => serde_json::Value::String(format!("{v}")),
            Self::StorageAt(v) => serde_json::Value::String(format!("0x{v:x}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Bytes;

    use super::Response;
    use crate::evm::forkdb::Request;

    /// Regression: `Response::to_json` for `Code` must not double-prefix `0x`.
    /// `alloy_primitives::Bytes` already includes the prefix in its `Display`
    /// implementation, so `to_json` must use `v.to_string()` instead of
    /// `format!("0x{v}")`. Otherwise the cached entry fails to parse on
    /// retrieval and is treated as stale, making the cache unusable.
    #[test]
    fn code_roundtrip_to_json() {
        let req = Request::GetCode {
            chain_id: 1,
            address: alloy_primitives::Address::ZERO,
            block: 1,
        };
        let raw = serde_json::Value::String("0x6000".into());

        let parsed = Response::parse(&req, &raw).expect("parsing valid code should succeed");
        let json = parsed.to_json();
        let s = json.as_str().expect("to_json must produce a string");

        assert!(
            s.starts_with("0x"),
            "code JSON must include the 0x prefix, got: {s}"
        );
        assert_eq!(
            s.matches("0x").count(),
            1,
            "code JSON must have exactly one 0x prefix, got: {s}"
        );

        let reparsed =
            Response::parse(&req, &json).expect("re-parsing cached code JSON should succeed");
        assert!(
            matches!(reparsed, Response::Code(ref b) if *b == Bytes::from_static(&[0x60, 0x00])),
            "roundtrip must preserve the same Bytes value"
        );
    }
}
