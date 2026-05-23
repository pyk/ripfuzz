//! Two-layer cache (in-memory + on-disk) for forked RPC responses.

use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use lru::LruCache;
use serde_json::Value;

use crate::evm::forkdb::request::Request;

const DEFAULT_MEMORY_CACHE_CAPACITY: usize = 4096;

/// Two-layer cache: in-memory LRU backed by individual JSON files per
/// request. Each entry is written atomically (temp file + rename) so there is
/// no race between parallel threads.
///
/// Files are stored under `{base_dir}/{request.cache_path()}`.
#[derive(Debug)]
pub struct Cache {
    base_dir: PathBuf,
    memory: Mutex<LruCache<String, Value>>,
    pub insert_count: AtomicUsize,
}

impl Cache {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self::with_capacity(base_dir, DEFAULT_MEMORY_CACHE_CAPACITY)
    }

    pub fn with_capacity(base_dir: impl AsRef<Path>, cap: usize) -> Self {
        let memory = match NonZeroUsize::new(cap) {
            Some(n) => LruCache::new(n),
            None => LruCache::unbounded(),
        };
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            memory: Mutex::new(memory),
            insert_count: AtomicUsize::new(0),
        }
    }

    fn cache_file_path(&self, req: &Request) -> PathBuf {
        self.base_dir.join(req.cache_path())
    }

    fn write_to_disk(&self, req: &Request, value: &Value) -> Result<()> {
        let path = self.cache_file_path(req);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(value)?;
        let temp = path.with_extension("tmp");
        fs::write(&temp, data)?;
        fs::rename(&temp, &path)?;
        Ok(())
    }

    /// Lookup in the in-memory cache first, then fall back to a lazy disk load.
    pub fn get(&self, req: &Request) -> Option<Value> {
        let key = req.cache_key();
        {
            let mut guard = self.memory.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(v) = guard.get(&key) {
                return Some(v.clone());
            }
        }

        let path = self.cache_file_path(req);
        if !path.exists() {
            return None;
        }

        let data = fs::read(&path).ok()?;
        let value: Value = serde_json::from_slice(&data).ok()?;

        let mut guard = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        guard.put(key, value.clone());

        Some(value)
    }

    /// Insert into memory and persist atomically to disk.
    pub fn insert(&self, req: &Request, value: Value) {
        self.insert_count.fetch_add(1, Ordering::SeqCst);
        let key = req.cache_key();
        {
            let mut guard = self.memory.lock().unwrap_or_else(|e| e.into_inner());
            guard.put(key, value.clone());
        }
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

        let req = Request::GetChainId;
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
            address: Address::ZERO,
            block: 1,
        };
        let req_b = Request::GetStorageAt {
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
            address: address!("0x0000000000000000000000000000000000000001"),
            slot: U256::from(42),
            block: 123,
        };

        cache.insert(&req, json!("0xabc"));

        let expected = tmp
            .path()
            .join("eth_getStorageAt/123/0000000000000000000000000000000000000001/2a.json");
        assert!(expected.exists(), "expected file at {expected:?}");
    }

    /// Regression: the in-memory cache must be bounded so that a long fuzzing
    /// campaign touching thousands of accounts and slots does not OOM.
    #[test]
    fn cache_memory_is_bounded_by_lru_capacity() {
        let tmp = tempdir().unwrap();
        let cache = Cache::with_capacity(tmp.path(), 2);

        let req1 = Request::GetChainId;
        let req2 = Request::GetBalance {
            address: Address::ZERO,
            block: 1,
        };
        let req3 = Request::GetBalance {
            address: Address::ZERO,
            block: 2,
        };

        cache.insert(&req1, json!("0x1"));
        cache.insert(&req2, json!("0x2"));
        cache.insert(&req3, json!("0x3"));

        assert_eq!(
            cache.memory.lock().unwrap_or_else(|e| e.into_inner()).len(),
            2,
            "memory cache must respect LRU capacity"
        );

        // Evicted entry must still be available from disk.
        assert_eq!(cache.get(&req1).unwrap(), "0x1");
        assert_eq!(cache.get(&req2).unwrap(), "0x2");
        assert_eq!(cache.get(&req3).unwrap(), "0x3");
    }
}
