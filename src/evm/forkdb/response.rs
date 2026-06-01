//! Strongly typed JSON-RPC responses for EVM state fetches.

use alloy_primitives::{Address, B256, Bytes, U64, U256};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::evm::forkdb::request::Request;

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
    pub fn parse(request: &Request, result: &serde_json::Value) -> Result<Self> {
        match request {
            Request::GetChainId { .. } => {
                let s = result.as_str().context("expected hex string for chainId")?;
                let v = U64::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16)
                    .context("invalid chainId hex")?;
                Ok(Self::ChainId(v.to()))
            }
            Request::GetBlockByNumber { .. } => {
                if result.is_null() {
                    bail!("block not found");
                }
                let block = Block::deserialize(result).context("invalid block response")?;
                Ok(Self::BlockByNumber(block))
            }
            Request::GetBalance { .. } => {
                let s = result.as_str().context("expected hex string for balance")?;
                let v: U256 = s.parse().context("invalid balance hex")?;
                Ok(Self::Balance(v))
            }
            Request::GetTransactionCount { .. } => {
                let s = result.as_str().context("expected hex string for nonce")?;
                let v = U64::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16)
                    .context("invalid nonce hex")?;
                Ok(Self::TransactionCount(v.to()))
            }
            Request::GetCode { .. } => {
                let s = result.as_str().context("expected hex string for code")?;
                let v: Bytes = s.parse().context("invalid code hex")?;
                Ok(Self::Code(v))
            }
            Request::GetStorageAt { .. } => {
                let s = result.as_str().context("expected hex string for storage")?;
                let v: U256 = s.parse().context("invalid storage hex")?;
                Ok(Self::StorageAt(v))
            }
        }
    }
}
