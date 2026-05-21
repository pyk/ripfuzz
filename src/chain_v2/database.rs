//! Unified database and fork backend for chain_v2.

use std::collections::HashMap;
use std::fs::{create_dir_all, read, write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use alloy_primitives::{Address, B256, U256};
use anyhow::{Context, Result, bail};
use revm::{
    DatabaseRef,
    bytecode::Bytecode,
    database::{CacheDB, InMemoryDB},
    database_interface::DBErrorMarker,
    state::AccountInfo,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, trace, warn};

use crate::rpc::RpcClient;

/// Unified EVM database.
#[derive(Clone, Debug)]
pub enum Database {
    Sandbox(InMemoryDB),
    Fork(CacheDB<ForkDb>),
}

impl Database {
    /// Insert or override account info.
    pub fn insert_account_info(&mut self, address: Address, info: AccountInfo) {
        match self {
            Self::Sandbox(db) => db.insert_account_info(address, info),
            Self::Fork(db) => db.insert_account_info(address, info),
        }
    }

    /// Insert a storage slot for an address.
    pub fn insert_account_storage(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> Result<(), DatabaseError> {
        match self {
            Self::Sandbox(db) => db
                .insert_account_storage(address, slot, value)
                .map_err(|e: std::convert::Infallible| match e {}),
            Self::Fork(db) => db
                .insert_account_storage(address, slot, value)
                .map_err(DatabaseError::from),
        }
    }
}

impl revm::DatabaseCommit for Database {
    fn commit(
        &mut self,
        changes: HashMap<Address, revm::state::Account, revm::primitives::map::FbBuildHasher<20>>,
    ) {
        match self {
            Self::Sandbox(db) => db.commit(changes),
            Self::Fork(db) => db.commit(changes),
        }
    }
}

impl revm::Database for Database {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Sandbox(db) => db.basic(address).map_err(|e| match e {}),
            Self::Fork(db) => db.basic(address).map_err(DatabaseError::from),
        }
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self {
            Self::Sandbox(db) => db.code_by_hash(code_hash).map_err(|e| match e {}),
            Self::Fork(db) => db.code_by_hash(code_hash).map_err(DatabaseError::from),
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Sandbox(db) => db.storage(address, index).map_err(|e| match e {}),
            Self::Fork(db) => db.storage(address, index).map_err(DatabaseError::from),
        }
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        match self {
            Self::Sandbox(db) => db.block_hash(number).map_err(|e| match e {}),
            Self::Fork(db) => db.block_hash(number).map_err(DatabaseError::from),
        }
    }
}

/// Snapshot of fork cache performance.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

impl Database {
    /// Return cache statistics if backed by a fork database.
    pub fn cache_stats(&self) -> Option<CacheStats> {
        match self {
            Self::Sandbox(_) => None,
            Self::Fork(db) => Some(db.db.cache_stats()),
        }
    }

    /// Flush the fork cache to disk, if any.
    pub fn flush_cache(&self) -> Result<()> {
        match self {
            Self::Sandbox(_) => Ok(()),
            Self::Fork(db) => db.db.flush_cache(),
        }
    }
}

/// Unified database error.
#[derive(Debug)]
pub enum DatabaseError {
    Fork(ForkDbError),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fork(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Fork(e) => Some(e),
        }
    }
}

impl DBErrorMarker for DatabaseError {}

impl From<ForkDbError> for DatabaseError {
    fn from(e: ForkDbError) -> Self {
        Self::Fork(e)
    }
}

impl From<std::convert::Infallible> for DatabaseError {
    fn from(e: std::convert::Infallible) -> Self {
        match e {}
    }
}

// ----------------------------------------------------------------------------
// ForkDb
// ----------------------------------------------------------------------------

/// Thin newtype around `anyhow::Error` so we can implement `DBErrorMarker`.
#[derive(Debug)]
pub struct ForkDbError(anyhow::Error);

impl std::fmt::Display for ForkDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ForkDbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl DBErrorMarker for ForkDbError {}

impl From<anyhow::Error> for ForkDbError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}

/// Remote + cached backend that satisfies `DatabaseRef`.
///
/// Cheaply cloneable because all state lives behind `Arc`.
#[derive(Clone, Debug)]
pub struct ForkDb {
    inner: Arc<ForkDbInner>,
}

#[derive(Debug)]
struct ForkDbInner {
    rpc: Arc<dyn RpcClient>,
    block_number: u64,
    account_cache: RwLock<HashMap<Address, CachedAccount>>,
    slot_cache: RwLock<HashMap<(Address, U256), U256>>,
    block_hash_cache: RwLock<HashMap<u64, B256>>,
    code_cache: RwLock<HashMap<B256, Bytecode>>,
    cache_file: PathBuf,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAccount {
    pub balance: U256,
    pub nonce: u64,
    pub code_hash: B256,
    pub code: Option<Vec<u8>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DiskCache {
    pub accounts: HashMap<Address, CachedAccount>,
    pub slots: Vec<((Address, U256), U256)>,
    pub block_hashes: HashMap<u64, B256>,
    pub code: HashMap<B256, Vec<u8>>,
}

impl ForkDb {
    pub fn new(
        rpc: Arc<dyn RpcClient>,
        block_number: u64,
        cache_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let cache_key = rpc.cache_key();
        let chain_id = cache_key.parse::<u64>().unwrap_or(0);
        let cache_file = cache_path(cache_dir.as_ref(), chain_id, block_number);

        let mut account_cache = HashMap::new();
        let mut slot_cache = HashMap::new();
        let mut block_hash_cache = HashMap::new();
        let mut code_cache = HashMap::new();

        if cache_file.exists() {
            trace!(?cache_file, "loading fork cache from disk");
            match read(&cache_file) {
                Ok(data) => match serde_json::from_slice::<DiskCache>(&data) {
                    Ok(cached) => {
                        for (addr, acc) in cached.accounts {
                            account_cache.insert(addr, acc);
                        }
                        for (key, value) in cached.slots {
                            slot_cache.insert(key, value);
                        }
                        for (num, hash) in cached.block_hashes {
                            block_hash_cache.insert(num, hash);
                        }
                        for (hash, code) in cached.code {
                            let bytecode = Bytecode::new_raw(alloy_primitives::Bytes::from(code));
                            code_cache.insert(hash, bytecode);
                        }
                        debug!(
                            accounts = account_cache.len(),
                            slots = slot_cache.len(),
                            "fork cache loaded from disk"
                        );
                    }
                    Err(e) => {
                        warn!(?cache_file, %e, "failed to deserialize fork cache, starting empty");
                    }
                },
                Err(e) => {
                    warn!(?cache_file, %e, "failed to read fork cache file, starting empty");
                }
            }
        }

        Ok(Self {
            inner: Arc::new(ForkDbInner {
                rpc,
                block_number,
                account_cache: RwLock::new(account_cache),
                slot_cache: RwLock::new(slot_cache),
                block_hash_cache: RwLock::new(block_hash_cache),
                code_cache: RwLock::new(code_cache),
                cache_file,
                cache_hits: AtomicU64::new(0),
                cache_misses: AtomicU64::new(0),
            }),
        })
    }

    /// Return current cache hit/miss counters.
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            hits: self.inner.cache_hits.load(Ordering::Relaxed),
            misses: self.inner.cache_misses.load(Ordering::Relaxed),
        }
    }

    /// Persist the current memory cache to disk.
    pub fn flush_cache(&self) -> Result<()> {
        let accounts = match self.inner.account_cache.read() {
            Ok(guard) => guard.clone(),
            Err(_) => bail!("account_cache lock poisoned"),
        };
        let slots = match self.inner.slot_cache.read() {
            Ok(guard) => guard.clone().into_iter().collect(),
            Err(_) => bail!("slot_cache lock poisoned"),
        };
        let block_hashes = match self.inner.block_hash_cache.read() {
            Ok(guard) => guard.clone(),
            Err(_) => bail!("block_hash_cache lock poisoned"),
        };
        let code = match self.inner.code_cache.read() {
            Ok(guard) => guard
                .iter()
                .map(|(h, b)| (*h, b.bytes().to_vec()))
                .collect(),
            Err(_) => bail!("code_cache lock poisoned"),
        };

        let disk = DiskCache {
            accounts,
            slots,
            block_hashes,
            code,
        };

        if let Some(parent) = self.inner.cache_file.parent() {
            create_dir_all(parent).context("creating fork cache directory")?;
        }
        let data = serde_json::to_vec_pretty(&disk).context("serializing fork cache")?;
        write(&self.inner.cache_file, data).context("writing fork cache file")?;
        debug!(?self.inner.cache_file, "fork cache flushed to disk");
        Ok(())
    }
}

impl DatabaseRef for ForkDb {
    type Error = ForkDbError;

    #[instrument(skip(self), fields(%address))]
    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        {
            let cache = self.inner.account_cache.read().map_err(lock_poisoned)?;
            if let Some(acc) = cache.get(&address) {
                trace!(%address, "account cache hit");
                self.inner.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(Some(AccountInfo {
                    balance: acc.balance,
                    nonce: acc.nonce,
                    code_hash: acc.code_hash,
                    code: acc
                        .code
                        .as_ref()
                        .cloned()
                        .map(|c| Bytecode::new_raw(alloy_primitives::Bytes::from(c))),
                    account_id: None,
                }));
            }
        }

        trace!(%address, "account cache miss — fetching from RPC");
        self.inner.cache_misses.fetch_add(1, Ordering::Relaxed);
        let fetched = self.inner.fetch_account(address)?;
        let info = AccountInfo {
            balance: fetched.balance,
            nonce: fetched.nonce,
            code_hash: fetched.code_hash,
            code: fetched
                .code
                .as_ref()
                .cloned()
                .map(|c| Bytecode::new_raw(alloy_primitives::Bytes::from(c))),
            account_id: None,
        };

        {
            let mut cache = self.inner.account_cache.write().map_err(lock_poisoned)?;
            cache.insert(address, fetched);
        }

        Ok(Some(info))
    }

    #[instrument(skip(self), fields(%code_hash))]
    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        {
            let cache = self.inner.code_cache.read().map_err(lock_poisoned)?;
            if let Some(code) = cache.get(&code_hash) {
                trace!(%code_hash, "code_by_hash cache hit");
                self.inner.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(code.clone());
            }
        }
        trace!(%code_hash, "code_by_hash cache miss — returning empty");
        self.inner.cache_misses.fetch_add(1, Ordering::Relaxed);
        warn!(%code_hash, "code_by_hash cache miss — returning empty bytecode");
        Ok(Bytecode::default())
    }

    #[instrument(skip(self), fields(%address, %index))]
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        {
            let cache = self.inner.slot_cache.read().map_err(lock_poisoned)?;
            if let Some(val) = cache.get(&(address, index)) {
                trace!(%address, %index, "slot cache hit");
                self.inner.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(*val);
            }
        }

        trace!(%address, %index, method = "eth_getStorageAt", "slot cache miss — fetching from RPC");
        self.inner.cache_misses.fetch_add(1, Ordering::Relaxed);
        let value = self.inner.fetch_slot(address, index)?;

        {
            let mut cache = self.inner.slot_cache.write().map_err(lock_poisoned)?;
            cache.insert((address, index), value);
        }

        Ok(value)
    }

    #[instrument(skip(self), fields(number))]
    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        {
            let cache = self.inner.block_hash_cache.read().map_err(lock_poisoned)?;
            if let Some(hash) = cache.get(&number) {
                trace!(number, "block hash cache hit");
                self.inner.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(*hash);
            }
        }
        trace!(
            number,
            method = "eth_getBlockByNumber",
            "block hash cache miss — fetching from RPC"
        );
        self.inner.cache_misses.fetch_add(1, Ordering::Relaxed);
        let hash = self.inner.fetch_block_hash(number)?;
        {
            let mut cache = self.inner.block_hash_cache.write().map_err(lock_poisoned)?;
            cache.insert(number, hash);
        }
        Ok(hash)
    }
}

impl ForkDbInner {
    #[instrument(skip(self), fields(%address))]
    fn fetch_account(&self, address: Address) -> Result<CachedAccount> {
        let addr_hex = format!("0x{address:x}");
        let block_hex = format!("0x{:x}", self.block_number);

        let balance = self.rpc.call(
            "eth_getBalance",
            &[
                serde_json::Value::String(addr_hex.clone()),
                serde_json::Value::String(block_hex.clone()),
            ],
        )?;
        let nonce = self.rpc.call(
            "eth_getTransactionCount",
            &[
                serde_json::Value::String(addr_hex.clone()),
                serde_json::Value::String(block_hex.clone()),
            ],
        )?;
        let code = self.rpc.call(
            "eth_getCode",
            &[
                serde_json::Value::String(addr_hex),
                serde_json::Value::String(block_hex),
            ],
        )?;

        let balance = parse_u256(&balance).unwrap_or_default();
        let nonce = parse_u64(&nonce).unwrap_or_default();
        let code_bytes = parse_hex_bytes(&code).unwrap_or_default();
        let bytecode = if code_bytes.is_empty() {
            Bytecode::default()
        } else {
            Bytecode::new_raw(alloy_primitives::Bytes::from(code_bytes.clone()))
        };

        let code_hash = bytecode.hash_slow();
        if !code_bytes.is_empty() {
            let mut code_cache = self.code_cache.write().map_err(lock_poisoned)?;
            code_cache.insert(code_hash, bytecode.clone());
        }

        Ok(CachedAccount {
            balance,
            nonce,
            code_hash,
            code: if code_bytes.is_empty() {
                None
            } else {
                Some(code_bytes)
            },
        })
    }

    #[instrument(skip(self), fields(%address, %slot))]
    fn fetch_slot(&self, address: Address, slot: U256) -> Result<U256> {
        let addr_hex = format!("0x{address:x}");
        let slot_hex = format!("0x{slot:x}");
        let block_hex = format!("0x{:x}", self.block_number);

        let result = self.rpc.call(
            "eth_getStorageAt",
            &[
                serde_json::Value::String(addr_hex),
                serde_json::Value::String(slot_hex),
                serde_json::Value::String(block_hex),
            ],
        )?;

        Ok(parse_u256(&result).unwrap_or_default())
    }

    #[instrument(skip(self), fields(number))]
    fn fetch_block_hash(&self, number: u64) -> Result<B256> {
        let block_hex = format!("0x{:x}", number);
        let result = self.rpc.call(
            "eth_getBlockByNumber",
            &[
                serde_json::Value::String(block_hex),
                serde_json::Value::Bool(false),
            ],
        )?;

        let hash = result
            .get("hash")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        Ok(hash)
    }
}

fn lock_poisoned<T>(_: std::sync::PoisonError<T>) -> anyhow::Error {
    anyhow::anyhow!("lock poisoned")
}

/// Derive the canonical cache path.
pub fn cache_path(cache_dir: impl AsRef<Path>, chain_id: u64, block_number: u64) -> PathBuf {
    cache_dir
        .as_ref()
        .join(format!("{}", chain_id))
        .join(format!("{}.json", block_number))
}

fn parse_u256(value: &serde_json::Value) -> Option<U256> {
    let s = value.as_str()?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    U256::from_str_radix(s, 16).ok()
}

fn parse_u64(value: &serde_json::Value) -> Option<u64> {
    let s = value.as_str()?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16).ok()
}

fn parse_hex_bytes(value: &serde_json::Value) -> Option<Vec<u8>> {
    let s = value.as_str()?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_derived_correctly() {
        let dir = std::path::Path::new("/tmp/proj/cache");
        let path = cache_path(dir, 1, 123);
        assert_eq!(path, PathBuf::from("/tmp/proj/cache/1/123.json"));
    }

    #[test]
    fn parse_u256_valid() {
        let v = serde_json::Value::String("0x1a2b".into());
        assert_eq!(parse_u256(&v).unwrap(), U256::from(0x1a2bu64));
    }

    #[test]
    fn parse_u64_valid() {
        let v = serde_json::Value::String("0x10".into());
        assert_eq!(parse_u64(&v).unwrap(), 16u64);
    }

    #[test]
    fn parse_hex_bytes_valid() {
        let v = serde_json::Value::String("0xabcd".into());
        assert_eq!(parse_hex_bytes(&v).unwrap(), vec![0xab, 0xcd]);
    }
}
