//! Two-layer cache: in-memory hashmap backed by individual JSON files per
//! request. Each entry is written atomically (temp file + rename) so there is
//! no race between parallel threads, even when they target the same key.
//!
//! Files are stored under `{base_dir}/{method}/` using structured paths
//! derived from the request parameters, e.g.
//! `{base_dir}/eth_getBlockByNumber/1234.json`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde_json::Value;

use crate::evm::forkdb::rpc::types::RpcRequest;

#[derive(Debug)]
pub struct Cache {
    base_dir: PathBuf,
    memory: RwLock<HashMap<String, Value>>,
}

impl Cache {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            memory: RwLock::new(HashMap::new()),
        }
    }

    /// Compute the on-disk path for a single request entry.
    fn disk_path(&self, request: &RpcRequest) -> PathBuf {
        let mut path = self.base_dir.clone();
        for component in request.cache_path_components() {
            path.push(component);
        }
        path
    }

    /// Atomically write one entry to its per-request file.
    fn write_to_disk(&self, request: &RpcRequest, value: &Value) {
        let path = self.disk_path(request);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let data = serde_json::to_vec_pretty(value).unwrap_or_default();
        let temp = path.with_extension("tmp");
        let _ = fs::write(&temp, data);
        let _ = fs::rename(&temp, &path);
    }

    /// Lookup in the in-memory cache first, then fall back to a lazy disk load.
    pub fn get(&self, request: &RpcRequest) -> Option<Value> {
        let key = request.cache_key();

        {
            let guard = self.memory.read().unwrap_or_else(|e| e.into_inner());
            if let Some(v) = guard.get(&key) {
                return Some(v.clone());
            }
        }

        let path = self.disk_path(request);
        if !path.exists() {
            return None;
        }

        let data = fs::read(&path).ok()?;
        let value: Value = serde_json::from_slice(&data).ok()?;

        let mut guard = self.memory.write().unwrap_or_else(|e| e.into_inner());
        guard.insert(key, value.clone());

        Some(value)
    }

    /// Insert into memory and persist atomically to disk.
    pub fn insert(&self, request: &RpcRequest, value: &serde_json::Value) {
        let key = request.cache_key();
        {
            let mut guard = self.memory.write().unwrap_or_else(|e| e.into_inner());
            guard.insert(key, value.clone());
        }
        self.write_to_disk(request, value);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use alloy_primitives::{Address, U256, address};
    use serde_json::json;

    use super::*;

    #[test]
    fn cache_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let key = RpcRequest::GetBlockByNumber { block: 1 };
        assert!(cache.get(&key).is_none());

        cache.insert(&key, &json!({"number": "0x1a2b"}));
        assert_eq!(cache.get(&key).unwrap(), json!({"number": "0x1a2b"}));

        // New instance should read from disk
        let cache2 = Cache::new(tmp.path());
        assert_eq!(cache2.get(&key).unwrap(), json!({"number": "0x1a2b"}));
    }

    #[test]
    fn cache_isolated_by_key() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let req_a = RpcRequest::GetBlockByNumber { block: 1 };
        let req_b = RpcRequest::GetBalance {
            address: Address::ZERO,
            block: 1,
        };

        let val_a: serde_json::Value = "0x1".into();
        let val_b: serde_json::Value = "0x2".into();
        cache.insert(&req_a, &val_a);
        cache.insert(&req_b, &val_b);

        assert_eq!(cache.get(&req_a).unwrap(), "0x1");
        assert_eq!(cache.get(&req_b).unwrap(), "0x2");
    }

    #[test]
    fn cache_path_for_storage_at() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let req = RpcRequest::GetStorageAt {
            address: address!("0x0000000000000000000000000000000000000001"),
            slot: U256::from(42),
            block: 1234,
        };

        cache.insert(&req, &json!("0xabc"));

        let expected = tmp
            .path()
            .join("eth_getStorageAt")
            .join("1234")
            .join("0x0000000000000000000000000000000000000001")
            .join("0x2a.json");
        assert!(expected.exists(), "expected cache file at {expected:?}");
    }

    // -----------------------------------------------------------------
    // Fixture-based cache read tests
    // -----------------------------------------------------------------

    /// Helper: copy a fixture file (full JSON-RPC response envelope)
    /// directly into the cache directory so `Cache::get` reads the exact
    /// wire-format response from disk.
    fn seed_fixture(cache: &Cache, request: &RpcRequest, fixture_path: impl AsRef<Path>) {
        let path = cache.disk_path(request);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::copy(fixture_path, path).unwrap();
    }

    /// Helper: extract `result` from a JSON-RPC response envelope.
    fn extract_result(envelope: &Value) -> Value {
        envelope.get("result").unwrap().clone()
    }

    #[test]
    fn cache_reads_fixture_eth_chainid() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let req = RpcRequest::ChainId;

        seed_fixture(&cache, &req, "fixtures/json-rpc-response/eth_chainId.json");

        assert_eq!(extract_result(&cache.get(&req).unwrap()), "0x1");
    }

    #[test]
    fn cache_reads_fixture_eth_getbalance() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let req = RpcRequest::GetBalance {
            address: Address::ZERO,
            block: 1,
        };
        let path = cache.disk_path(&req);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::copy("fixtures/json-rpc-response/eth_getBalance.json", &path).unwrap();

        assert_eq!(
            extract_result(&cache.get(&req).unwrap()),
            "0x4ec7cefe1a0664fd"
        );
    }

    #[test]
    fn cache_reads_fixture_eth_getblockbynumber_concrete() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let req = RpcRequest::GetBlockByNumber { block: 0x17fa30b };
        let path = cache.disk_path(&req);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::copy(
            "fixtures/json-rpc-response/eth_getBlockByNumber_block_false.json",
            &path,
        )
        .unwrap();

        let result = extract_result(&cache.get(&req).unwrap());
        let block = result.as_object().unwrap();
        assert_eq!(block.get("number").unwrap(), "0x17fa30b");
        assert_eq!(block.get("difficulty").unwrap(), "0x0");
        assert!(
            block
                .get("hash")
                .unwrap()
                .as_str()
                .unwrap()
                .starts_with("0x")
        );
    }
}
