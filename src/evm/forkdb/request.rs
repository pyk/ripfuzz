//! Strongly typed JSON-RPC requests for EVM state fetches.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use alloy_primitives::{Address, U256};
use serde_json::json;

/// Strongly typed JSON-RPC request for a single EVM state fetch.
///
/// Every state-fetch variant carries `chain_id` so that the on-disk cache is
/// isolated per chain. `GetChainId` is special: it carries `url_hash` so
/// that the cache entry is scoped to the RPC endpoint without needing the
/// chain identifier up front.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Request {
    GetChainId {
        url_hash: u64,
    },
    GetBlockByNumber {
        chain_id: u64,
        block: u64,
        full_tx: bool,
    },
    GetBalance {
        chain_id: u64,
        address: Address,
        block: u64,
    },
    GetTransactionCount {
        chain_id: u64,
        address: Address,
        block: u64,
    },
    GetCode {
        chain_id: u64,
        address: Address,
        block: u64,
    },
    GetStorageAt {
        chain_id: u64,
        address: Address,
        slot: U256,
        block: u64,
    },
}

impl Request {
    /// JSON-RPC method name.
    pub fn method(&self) -> &'static str {
        match self {
            Self::GetChainId { .. } => "eth_chainId",
            Self::GetBlockByNumber { .. } => "eth_getBlockByNumber",
            Self::GetBalance { .. } => "eth_getBalance",
            Self::GetTransactionCount { .. } => "eth_getTransactionCount",
            Self::GetCode { .. } => "eth_getCode",
            Self::GetStorageAt { .. } => "eth_getStorageAt",
        }
    }

    /// JSON-RPC parameters as serde values.
    pub fn params(&self) -> Vec<serde_json::Value> {
        match self {
            Self::GetChainId { .. } => vec![],
            Self::GetBlockByNumber { block, full_tx, .. } => {
                vec![json!(format!("0x{block:x}")), json!(*full_tx)]
            }
            Self::GetBalance { address, block, .. } => {
                vec![
                    json!(format!("0x{address:x}")),
                    json!(format!("0x{block:x}")),
                ]
            }
            Self::GetTransactionCount { address, block, .. } => {
                vec![
                    json!(format!("0x{address:x}")),
                    json!(format!("0x{block:x}")),
                ]
            }
            Self::GetCode { address, block, .. } => {
                vec![
                    json!(format!("0x{address:x}")),
                    json!(format!("0x{block:x}")),
                ]
            }
            Self::GetStorageAt {
                address,
                slot,
                block,
                ..
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
            Self::GetChainId { url_hash } => format!("eth_chainId/{url_hash:x}"),
            Self::GetBlockByNumber {
                chain_id,
                block,
                full_tx,
            } => {
                format!("eth_getBlockByNumber/{chain_id}/{block}/{}", *full_tx as u8)
            }
            Self::GetBalance {
                chain_id,
                address,
                block,
            } => {
                format!("eth_getBalance/{chain_id}/{block}/{address:x}")
            }
            Self::GetTransactionCount {
                chain_id,
                address,
                block,
            } => {
                format!("eth_getTransactionCount/{chain_id}/{block}/{address:x}")
            }
            Self::GetCode {
                chain_id,
                address,
                block,
            } => {
                format!("eth_getCode/{chain_id}/{block}/{address:x}")
            }
            Self::GetStorageAt {
                chain_id,
                address,
                slot,
                block,
            } => {
                format!("eth_getStorageAt/{chain_id}/{block}/{address:x}/{slot:x}")
            }
        }
    }

    /// Relative file path under the cache directory.
    pub fn cache_path(&self) -> PathBuf {
        PathBuf::from(self.cache_key()).with_extension("json")
    }
}

/// Deterministic hash of an RPC URL, used as the `eth_chainId` cache key.
pub fn url_hash(url: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::Request;

    #[test]
    fn get_block_by_number_cache_key_includes_full_tx() {
        let block = 1_234_567u64;
        let req_false = Request::GetBlockByNumber {
            chain_id: 1,
            block,
            full_tx: false,
        };
        let req_true = Request::GetBlockByNumber {
            chain_id: 1,
            block,
            full_tx: true,
        };
        let key_false = req_false.cache_key();
        let key_true = req_true.cache_key();
        assert_ne!(
            key_false, key_true,
            "cache keys must differ when full_tx differs (got {key_false})"
        );
    }
}
