//! Two-layer cache: in-memory hashmap backed by a JSON file on disk.

use std::collections::HashMap;
use std::fs::{create_dir_all, read, write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::rpc_v2::RequestKey;

#[derive(Debug)]
pub struct Cache {
    memory: RwLock<HashMap<RequestKey, serde_json::Value>>,
    disk_file: PathBuf,
    dirty: RwLock<bool>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DiskCache {
    entries: HashMap<String, serde_json::Value>,
}

impl Cache {
    pub fn new(disk_file: impl AsRef<Path>) -> Self {
        let disk_file = disk_file.as_ref().to_path_buf();
        let memory = Self::load(&disk_file).unwrap_or_default();
        Self {
            memory: RwLock::new(memory),
            disk_file,
            dirty: RwLock::new(false),
        }
    }

    fn load(path: impl AsRef<Path>) -> Result<HashMap<RequestKey, serde_json::Value>> {
        let data = read(path.as_ref()).context("reading disk cache")?;
        let disk: DiskCache = serde_json::from_slice(&data).context("parsing disk cache")?;
        let mut map = HashMap::with_capacity(disk.entries.len());
        for (k, v) in disk.entries {
            let key: RequestKey = serde_json::from_str(&k).context("parsing cache key")?;
            map.insert(key, v);
        }
        Ok(map)
    }

    /// Lookup in the unified memory cache (seeded from disk at construction).
    pub fn get(&self, key: &RequestKey) -> Option<serde_json::Value> {
        let guard = self.memory.read().unwrap_or_else(|e| e.into_inner());
        guard.get(key).cloned()
    }

    /// Insert into memory and mark the disk layer dirty.
    pub fn insert(&self, key: RequestKey, value: serde_json::Value) {
        let mut guard = self.memory.write().unwrap_or_else(|e| e.into_inner());
        guard.insert(key, value);
        let mut dirty = self.dirty.write().unwrap_or_else(|e| e.into_inner());
        *dirty = true;
    }

    /// Persist the memory cache to disk if dirty.
    pub fn flush(&self) -> Result<()> {
        if !*self.dirty.read().unwrap_or_else(|e| e.into_inner()) {
            return Ok(());
        }
        let snapshot = {
            let guard = self.memory.read().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        let entries: HashMap<String, serde_json::Value> = snapshot
            .into_iter()
            .map(|(k, v)| -> Result<(String, serde_json::Value)> {
                let key_json = serde_json::to_string(&k).context("serializing cache key")?;
                Ok((key_json, v))
            })
            .collect::<Result<HashMap<String, serde_json::Value>>>()?;
        let disk = DiskCache { entries };
        let data = serde_json::to_vec_pretty(&disk).context("serializing disk cache")?;
        if let Some(parent) = self.disk_file.parent() {
            create_dir_all(parent).context("creating cache directory")?;
        }
        write(&self.disk_file, data).context("writing disk cache")?;
        *self.dirty.write().unwrap_or_else(|e| e.into_inner()) = false;
        Ok(())
    }
}
