use std::path::PathBuf;

use alloy_primitives::{Address, U256};

/// Strongly typed JSON-RPC request for a single EVM state fetch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Request {
    GetChainId,
    GetBlockByNumber {
        block: u64,
        full_tx: bool,
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

impl Request {
    /// JSON-RPC method name.
    pub fn method(&self) -> &'static str {
        match self {
            Self::GetChainId => "eth_chainId",
            Self::GetBlockByNumber { .. } => "eth_getBlockByNumber",
            Self::GetBalance { .. } => "eth_getBalance",
            Self::GetTransactionCount { .. } => "eth_getTransactionCount",
            Self::GetCode { .. } => "eth_getCode",
            Self::GetStorageAt { .. } => "eth_getStorageAt",
        }
    }

    /// JSON-RPC parameters as serde values.
    pub fn params(&self) -> Vec<serde_json::Value> {
        use serde_json::json;
        match self {
            Self::GetChainId => vec![],
            Self::GetBlockByNumber { block, full_tx } => {
                vec![json!(format!("0x{block:x}")), json!(*full_tx)]
            }
            Self::GetBalance { address, block } => {
                vec![
                    json!(format!("0x{address:x}")),
                    json!(format!("0x{block:x}")),
                ]
            }
            Self::GetTransactionCount { address, block } => {
                vec![
                    json!(format!("0x{address:x}")),
                    json!(format!("0x{block:x}")),
                ]
            }
            Self::GetCode { address, block } => {
                vec![
                    json!(format!("0x{address:x}")),
                    json!(format!("0x{block:x}")),
                ]
            }
            Self::GetStorageAt {
                address,
                slot,
                block,
            } => {
                vec![
                    json!(format!("0x{address:x}")),
                    json!(format!("0x{slot:x}")),
                    json!(format!("0x{block:x}")),
                ]
            }
        }
    }

    /// Unique cache key used for in-memory lookup and disk path.
    pub fn cache_key(&self) -> String {
        match self {
            Self::GetChainId => "eth_chainId".into(),
            Self::GetBlockByNumber { block, .. } => {
                format!("eth_getBlockByNumber/{block}")
            }
            Self::GetBalance { address, block } => {
                format!("eth_getBalance/{block}/{address:x}")
            }
            Self::GetTransactionCount { address, block } => {
                format!("eth_getTransactionCount/{block}/{address:x}")
            }
            Self::GetCode { address, block } => {
                format!("eth_getCode/{block}/{address:x}")
            }
            Self::GetStorageAt {
                address,
                slot,
                block,
            } => {
                format!("eth_getStorageAt/{block}/{address:x}/{slot:x}")
            }
        }
    }

    /// Relative file path under the cache directory.
    pub fn cache_path(&self) -> PathBuf {
        PathBuf::from(self.cache_key()).with_extension("json")
    }
}
