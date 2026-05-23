//! Two-layer cache (in-memory + on-disk) for forked RPC responses.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use dashmap::DashMap;
use serde_json::Value;
use walkdir::WalkDir;

use crate::evm::forkdb::request::Request;

/// Two-layer cache: in-memory map backed by individual JSON files per
/// request. Each entry is written atomically (temp file + rename) so there is
/// no race between parallel threads.
///
/// Files are stored under `{base_dir}/{request.cache_path()}`.
#[derive(Debug)]
pub struct Cache {
    base_dir: PathBuf,
    memory: DashMap<String, Value>,
    pub insert_count: AtomicUsize,
}

impl Cache {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        let cache = Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            memory: DashMap::new(),
            insert_count: AtomicUsize::new(0),
        };
        cache.load_from_disk();
        cache
    }

    pub fn with_capacity(base_dir: impl AsRef<Path>, _cap: usize) -> Self {
        // Capacity is ignored; DashMap grows unbounded.
        Self::new(base_dir)
    }

    fn cache_file_path(&self, req: &Request) -> PathBuf {
        self.base_dir.join(req.cache_path())
    }

    fn write_to_disk(&self, req: &Request, value: &Value) -> Result<()> {
        let path = self.cache_file_path(req);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec(value)?;
        let temp = path.with_extension("tmp");
        fs::write(&temp, data)?;
        fs::rename(&temp, &path)?;
        Ok(())
    }

    /// Load all existing `.json` files from the base directory into the
    /// in-memory map so that `get` never touches the filesystem.
    fn load_from_disk(&self) {
        for entry in WalkDir::new(&self.base_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            if let Ok(data) = fs::read(path)
                && let Ok(value) = serde_json::from_slice::<Value>(&data)
                && let Ok(relative) = path.strip_prefix(&self.base_dir)
            {
                let key = relative.to_string_lossy();
                let key = match key.strip_suffix(".json") {
                    Some(k) => k,
                    None => &key,
                };
                self.memory.insert(key.into(), value);
            }
        }
    }

    /// Lookup in the in-memory cache only. The disk is never touched on the
    /// hot path; all persisted entries are loaded into memory when the Cache
    /// is constructed.
    pub fn get(&self, req: &Request) -> Option<Value> {
        let key = req.cache_key();
        self.memory.get(&key).as_deref().cloned()
    }

    /// Insert into memory and persist atomically to disk.
    pub fn insert(&self, req: &Request, value: Value) {
        self.insert_count.fetch_add(1, Ordering::SeqCst);
        let key = req.cache_key();
        self.memory.insert(key, value.clone());
        let _ = self.write_to_disk(req, &value);
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256, address};
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn cache_roundtrip() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let req = Request::GetChainId { url_hash: 0xabc };
        assert!(cache.get(&req).is_none());

        cache.insert(&req, json!("0x1"));
        assert_eq!(cache.get(&req).unwrap(), "0x1");

        // New instance reads from disk
        let cache2 = Cache::new(tmp.path());
        assert_eq!(cache2.get(&req).unwrap(), "0x1");
    }

    #[test]
    fn cache_isolated_by_request() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let req_a = Request::GetBalance {
            chain_id: 1,
            address: Address::ZERO,
            block: 1,
        };
        let req_b = Request::GetStorageAt {
            chain_id: 1,
            address: Address::ZERO,
            slot: U256::from(2),
            block: 1,
        };

        cache.insert(&req_a, "0x1".into());
        cache.insert(&req_b, "0x2".into());

        assert_eq!(cache.get(&req_a).unwrap(), "0x1");
        assert_eq!(cache.get(&req_b).unwrap(), "0x2");
    }

    #[test]
    fn cache_disk_layout_matches_request_path() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let req = Request::GetStorageAt {
            chain_id: 1,
            address: address!("0x0000000000000000000000000000000000000001"),
            slot: U256::from(42),
            block: 123,
        };

        cache.insert(&req, json!("0xabc"));

        let expected = tmp
            .path()
            .join("eth_getStorageAt/1/123/0000000000000000000000000000000000000001/2a.json");
        assert!(expected.exists(), "expected file at {expected:?}");
    }

    /// Regression: disk cache must use compact JSON, not pretty-printed JSON.
    #[test]
    fn cache_disk_uses_compact_json() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let req = Request::GetStorageAt {
            chain_id: 1,
            address: Address::ZERO,
            slot: U256::from(42),
            block: 1,
        };
        let value = json!({"a": 1, "b": [2, 3]});

        cache.insert(&req, value.clone());

        let expected = tmp
            .path()
            .join("eth_getStorageAt/1/1/0000000000000000000000000000000000000000/2a.json");
        let on_disk = fs::read(&expected).unwrap();
        let compact = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            on_disk, compact,
            "disk cache must use compact JSON instead of pretty-printed JSON"
        );
    }

    /// Regression: the hot path must never touch the filesystem. A new Cache
    /// instance loads disk contents into memory at construction time; after
    /// that, `get` must remain a pure memory operation.
    #[test]
    fn cache_get_never_reads_disk_after_construction() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path());

        let req = Request::GetChainId { url_hash: 0xabc };
        cache.insert(&req, json!("0x1"));

        // New instance loads from disk eagerly during construction.
        let cache2 = Cache::new(tmp.path());
        // Remove the backing directory so any disk read would fail.
        fs::remove_dir_all(tmp.path()).unwrap();

        // Must still succeed because the value is fully in memory.
        assert_eq!(cache2.get(&req).unwrap(), "0x1");
    }

    /// Regression: two chains sharing the same cache_dir must not see each
    /// other's cached state entries.
    #[test]
    fn cache_chain_id_isolation() {
        let tmp = tempdir().unwrap();

        let cache1 = Cache::new(tmp.path());
        let req = Request::GetBalance {
            chain_id: 1,
            address: Address::ZERO,
            block: 100,
        };
        cache1.insert(&req, json!("0xabc"));

        let cache2 = Cache::new(tmp.path());
        let req2 = Request::GetBalance {
            chain_id: 8453,
            address: Address::ZERO,
            block: 100,
        };

        assert!(
            cache2.get(&req2).is_none(),
            "chain 8453 must not see chain 1 cache entry"
        );
    }
}
