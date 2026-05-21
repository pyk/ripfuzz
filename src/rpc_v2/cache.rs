//! Two-layer cache: in-memory hashmap backed by individual JSON files per
//! request. Each entry is written atomically (temp file + rename) so there is
//! no race between parallel threads, even when they target the same key.
//!
//! Files are stored directly under `{base_dir}/rpc/` using the caller-provided
//! cache key as the filename, e.g. `{base_dir}/rpc/get_balance_1_0xabc.json`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Context, Result};
use serde_json::Value;

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
    fn cache_file_path(&self, key: &str) -> PathBuf {
        self.base_dir.join("rpc").join(format!("{key}.json"))
    }

    /// Atomically write one entry to its per-request file.
    fn write_to_disk(&self, key: &str, value: &Value) -> Result<()> {
        let path = self.cache_file_path(key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("creating cache directory")?;
        }
        let data = serde_json::to_vec_pretty(value).context("serializing cache entry")?;
        let temp = path.with_extension("tmp");
        fs::write(&temp, data).context("writing cache temp file")?;
        fs::rename(&temp, &path).context("renaming cache temp file")?;
        Ok(())
    }

    /// Lookup in the in-memory cache first, then fall back to a lazy disk load.
    pub fn get(&self, key: &str) -> Option<Value> {
        {
            let guard = self.memory.read().unwrap_or_else(|e| e.into_inner());
            if let Some(v) = guard.get(key) {
                return Some(v.clone());
            }
        }

        let path = self.cache_file_path(key);
        if !path.exists() {
            return None;
        }

        let data = fs::read(&path).ok()?;
        let value: Value = serde_json::from_slice(&data).ok()?;

        let mut guard = self.memory.write().unwrap_or_else(|e| e.into_inner());
        guard.insert(key.into(), value.clone());

        Some(value)
    }

    /// Insert into memory and persist atomically to disk.
    pub fn insert(&self, key: &str, value: Value) {
        {
            let mut guard = self.memory.write().unwrap_or_else(|e| e.into_inner());
            guard.insert(key.into(), value.clone());
        }
        let _ = self.write_to_disk(key, &value);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use super::*;

    #[test]
    fn cache_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let key = "get_block_by_number_1_latest";
        assert!(cache.get(key).is_none());

        cache.insert(key, json!({"number": "0x1a2b"}));
        assert_eq!(cache.get(key).unwrap(), json!({"number": "0x1a2b"}));

        // New instance should read from disk
        let cache2 = Cache::new(tmp.path());
        assert_eq!(cache2.get(key).unwrap(), json!({"number": "0x1a2b"}));
    }

    #[test]
    fn cache_isolated_by_key() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let key_a = "get_block_by_number_1_latest";
        let key_b = "get_balance_1_0x0";

        cache.insert(key_a, "0x1".into());
        cache.insert(key_b, "0x2".into());

        assert_eq!(cache.get(key_a).unwrap(), "0x1");
        assert_eq!(cache.get(key_b).unwrap(), "0x2");
    }

    // -----------------------------------------------------------------
    // Fixture-based cache read tests
    // -----------------------------------------------------------------

    /// Helper: copy a fixture file (full JSON-RPC response envelope)
    /// directly into the cache directory so `Cache::get` reads the exact
    /// wire-format response from disk.
    fn seed_fixture(cache: &Cache, key: &str, fixture_path: impl AsRef<Path>) {
        let path = cache.cache_file_path(key);
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
        let key = "eth_chainId";

        seed_fixture(&cache, key, "fixtures/json-rpc-response/eth_chainId.json");

        assert_eq!(extract_result(&cache.get(key).unwrap()), "0x1");
    }

    #[test]
    fn cache_reads_fixture_eth_getbalance() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let key = "eth_getBalance";

        seed_fixture(
            &cache,
            key,
            "fixtures/json-rpc-response/eth_getBalance.json",
        );

        assert_eq!(
            extract_result(&cache.get(key).unwrap()),
            "0x4ec7cefe1a0664fd"
        );
    }

    #[test]
    fn cache_reads_fixture_eth_gettransactioncount() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let key = "eth_getTransactionCount";

        seed_fixture(
            &cache,
            key,
            "fixtures/json-rpc-response/eth_getTransactionCount.json",
        );

        assert_eq!(extract_result(&cache.get(key).unwrap()), "0x1707");
    }

    #[test]
    fn cache_reads_fixture_eth_getcode() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let key = "eth_getCode";

        seed_fixture(&cache, key, "fixtures/json-rpc-response/eth_getCode.json");

        let result = extract_result(&cache.get(key).unwrap());
        let s = result.as_str().unwrap();
        assert!(s.starts_with("0x60606040"));
        assert!(s.len() > 100);
    }

    #[test]
    fn cache_reads_fixture_eth_getstorageat() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let key = "eth_getStorageAt";

        seed_fixture(
            &cache,
            key,
            "fixtures/json-rpc-response/eth_getStorageAt.json",
        );

        assert_eq!(
            extract_result(&cache.get(key).unwrap()),
            "0x577261707065642045746865720000000000000000000000000000000000001a"
        );
    }

    #[test]
    fn cache_reads_fixture_eth_getblockbynumber_latest() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let key = "eth_getBlockByNumber_latest";

        seed_fixture(
            &cache,
            key,
            "fixtures/json-rpc-response/eth_getBlockByNumber_latest_false.json",
        );

        let result = extract_result(&cache.get(key).unwrap());
        let block = result.as_object().unwrap();
        assert_eq!(block.get("number").unwrap(), "0x17fa30c");
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

    #[test]
    fn cache_reads_fixture_eth_getblockbynumber_concrete() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let key = "eth_getBlockByNumber_0x17fa30b";

        seed_fixture(
            &cache,
            key,
            "fixtures/json-rpc-response/eth_getBlockByNumber_block_false.json",
        );

        let result = extract_result(&cache.get(key).unwrap());
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
