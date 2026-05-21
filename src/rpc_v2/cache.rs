//! Two-layer cache: in-memory hashmap backed by individual JSON files per
//! request. Each entry is written atomically (temp file + rename) so there is
//! no race between parallel threads, even when they target the same key.

use std::collections::HashMap;
use std::fs::{create_dir_all, read, write};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::rpc_v2::RequestKey;

#[derive(Debug)]
pub struct Cache {
    base_dir: PathBuf,
    chain_id: u64,
    memory: RwLock<HashMap<RequestKey, Value>>,
}

impl Cache {
    pub fn new(base_dir: impl AsRef<Path>, chain_id: u64) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            chain_id,
            memory: RwLock::new(HashMap::new()),
        }
    }

    /// Compute the on-disk path for a single request entry.
    fn cache_file_path(&self, key: &RequestKey) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let hash = format!("{:x}", hasher.finish());
        self.base_dir
            .join("rpc")
            .join(key.method())
            .join(format!("{}_{hash}.json", self.chain_id))
    }

    /// Atomically write one entry to its per-request file.
    fn write_to_disk(&self, key: &RequestKey, value: &Value) -> Result<()> {
        let path = self.cache_file_path(key);
        if let Some(parent) = path.parent() {
            create_dir_all(parent).context("creating cache directory")?;
        }
        let data = serde_json::to_vec_pretty(value).context("serializing cache entry")?;
        let temp = path.with_extension("tmp");
        write(&temp, data).context("writing cache temp file")?;
        std::fs::rename(&temp, &path).context("renaming cache temp file")?;
        Ok(())
    }

    /// Lookup in the in-memory cache first, then fall back to a lazy disk load.
    pub fn get(&self, key: &RequestKey) -> Option<Value> {
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

        let data = read(&path).ok()?;
        let value: Value = serde_json::from_slice(&data).ok()?;

        let mut guard = self.memory.write().unwrap_or_else(|e| e.into_inner());
        guard.insert(key.clone(), value.clone());

        Some(value)
    }

    /// Insert into memory and persist atomically to disk.
    pub fn insert(&self, key: RequestKey, value: Value) {
        {
            let mut guard = self.memory.write().unwrap_or_else(|e| e.into_inner());
            guard.insert(key.clone(), value.clone());
        }
        let _ = self.write_to_disk(&key, &value);
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path(), 1);

        let key = RequestKey::new("eth_blockNumber", &[]);
        assert!(cache.get(&key).is_none());

        cache.insert(key.clone(), "0x1a2b".into());
        assert_eq!(cache.get(&key).unwrap(), "0x1a2b");

        // New instance should read from disk
        let cache2 = Cache::new(tmp.path(), 1);
        assert_eq!(cache2.get(&key).unwrap(), "0x1a2b");
    }

    #[test]
    fn cache_isolated_by_method() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path(), 1);

        let key_a = RequestKey::new("eth_blockNumber", &[]);
        let key_b = RequestKey::new("eth_getBalance", &[]);

        cache.insert(key_a.clone(), "0x1".into());
        cache.insert(key_b.clone(), "0x2".into());

        assert_eq!(cache.get(&key_a).unwrap(), "0x1");
        assert_eq!(cache.get(&key_b).unwrap(), "0x2");
    }

    #[test]
    fn cache_isolated_by_chain_id() {
        let tmp = tempfile::tempdir().unwrap();
        let cache1 = Cache::new(tmp.path(), 1);
        let cache2 = Cache::new(tmp.path(), 56);

        let key = RequestKey::new("eth_blockNumber", &[]);
        cache1.insert(key.clone(), "0x1".into());

        // Same key, different chain_id => different file => miss
        assert!(cache2.get(&key).is_none());
    }
}
