//! Strongly typed request and response types for each JSON-RPC method.

use alloy_primitives::{Address, B256, Bytes, U64, U256};
use serde::Deserialize;

/// Chain info returned by [`Client::get_remote_chain_info`](super::Client).
#[derive(Debug, Clone)]
pub struct RemoteChainInfo {
    pub chain_id: u64,
    pub block: RemoteBlockInfo,
}

/// Block info returned by [`Client::get_remote_block_info`](super::Client).
#[derive(Debug, Clone)]
pub struct RemoteBlockInfo {
    pub number: u64,
    pub timestamp: u64,
    pub coinbase: Address,
    pub gas_limit: u64,
    pub basefee: u64,
    pub difficulty: U256,
    pub prevrandao: Option<B256>,
    pub excess_blob_gas: Option<u64>,
    pub hash: Option<B256>,
}

/// Account info returned by [`Client::get_remote_account_info`](super::Client).
#[derive(Debug, Clone)]
pub struct RemoteAccountInfo {
    pub balance: U256,
    pub nonce: u64,
    pub code: Bytes,
}

/// Typed response for `eth_chainId`.
#[derive(Debug, Clone)]
pub struct ChainIdResponse(pub u64);

/// Typed response for `eth_getBlockByNumber`.
#[derive(Debug, Clone, Deserialize)]
pub struct GetBlockByNumberResponse {
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

impl From<GetBlockByNumberResponse> for RemoteBlockInfo {
    fn from(r: GetBlockByNumberResponse) -> Self {
        Self {
            number: r.number.to(),
            timestamp: r.timestamp.to(),
            coinbase: r.coinbase,
            gas_limit: r.gas_limit.to(),
            basefee: r.basefee.to(),
            difficulty: r.difficulty,
            prevrandao: r.prevrandao,
            excess_blob_gas: r.excess_blob_gas.map(|v| v.to()),
            hash: r.hash,
        }
    }
}

/// Typed response for `eth_getBalance`.
#[derive(Debug, Clone)]
pub struct GetBalanceResponse(pub U256);

/// Typed response for `eth_getTransactionCount`.
#[derive(Debug, Clone)]
pub struct GetTransactionCountResponse(pub u64);

/// Typed response for `eth_getCode`.
#[derive(Debug, Clone)]
pub struct GetCodeResponse(pub Bytes);

/// Typed response for `eth_getStorageAt`.
#[derive(Debug, Clone)]
pub struct GetStorageAtResponse(pub U256);

/// Internal discriminant for every cacheable RPC request.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum RpcRequest {
    ChainId,
    GetBlockByNumber {
        block: u64,
    },
    GetBalance {
        address: Address,
        block: u64,
    },
    GetTransactionCount {
        address: Address,
        block: u64,
    },
    GetCode {
        address: Address,
        block: u64,
    },
    GetStorageAt {
        address: Address,
        slot: U256,
        block: u64,
    },
}

impl RpcRequest {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::ChainId => "eth_chainId",
            Self::GetBlockByNumber { .. } => "eth_getBlockByNumber",
            Self::GetBalance { .. } => "eth_getBalance",
            Self::GetTransactionCount { .. } => "eth_getTransactionCount",
            Self::GetCode { .. } => "eth_getCode",
            Self::GetStorageAt { .. } => "eth_getStorageAt",
        }
    }

    pub fn to_json_params(&self) -> Vec<serde_json::Value> {
        match self {
            Self::ChainId => vec![],
            Self::GetBlockByNumber { block } => {
                vec![
                    serde_json::json!(format!("0x{block:x}")),
                    serde_json::json!(false),
                ]
            }
            Self::GetBalance { address, block } => {
                vec![
                    serde_json::json!(format!("0x{address:x}")),
                    serde_json::json!(format!("0x{block:x}")),
                ]
            }
            Self::GetTransactionCount { address, block } => {
                vec![
                    serde_json::json!(format!("0x{address:x}")),
                    serde_json::json!(format!("0x{block:x}")),
                ]
            }
            Self::GetCode { address, block } => {
                vec![
                    serde_json::json!(format!("0x{address:x}")),
                    serde_json::json!(format!("0x{block:x}")),
                ]
            }
            Self::GetStorageAt {
                address,
                slot,
                block,
            } => {
                vec![
                    serde_json::json!(format!("0x{address:x}")),
                    serde_json::json!(format!("0x{slot:x}")),
                    serde_json::json!(format!("0x{block:x}")),
                ]
            }
        }
    }

    pub fn to_json_payload(&self, id: u64) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": self.method_name(),
            "params": self.to_json_params(),
        })
    }

    /// Canonical string key used for in-memory cache and deduplication.
    pub fn cache_key(&self) -> String {
        match self {
            Self::ChainId => "eth_chainId".into(),
            Self::GetBlockByNumber { block } => format!("eth_getBlockByNumber:{block}"),
            Self::GetBalance { address, block } => {
                format!("eth_getBalance:{block}:0x{address:x}")
            }
            Self::GetTransactionCount { address, block } => {
                format!("eth_getTransactionCount:{block}:0x{address:x}")
            }
            Self::GetCode { address, block } => format!("eth_getCode:{block}:0x{address:x}"),
            Self::GetStorageAt {
                address,
                slot,
                block,
            } => {
                format!("eth_getStorageAt:{block}:0x{address:x}:0x{slot:x}")
            }
        }
    }

    /// Relative path components under the cache directory.
    pub fn cache_path_components(&self) -> Vec<String> {
        match self {
            Self::ChainId => vec!["eth_chainId".into(), "chain_id.json".into()],
            Self::GetBlockByNumber { block } => {
                vec!["eth_getBlockByNumber".into(), format!("{block}.json")]
            }
            Self::GetBalance { address, block } => {
                vec![
                    "eth_getBalance".into(),
                    format!("{block}"),
                    format!("0x{address:x}.json"),
                ]
            }
            Self::GetTransactionCount { address, block } => {
                vec![
                    "eth_getTransactionCount".into(),
                    format!("{block}"),
                    format!("0x{address:x}.json"),
                ]
            }
            Self::GetCode { address, block } => {
                vec![
                    "eth_getCode".into(),
                    format!("{block}"),
                    format!("0x{address:x}.json"),
                ]
            }
            Self::GetStorageAt {
                address,
                slot,
                block,
            } => {
                vec![
                    "eth_getStorageAt".into(),
                    format!("{block}"),
                    format!("0x{address:x}"),
                    format!("0x{slot:x}.json"),
                ]
            }
        }
    }
}
