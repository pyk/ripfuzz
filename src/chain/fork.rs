//! Fork network support: lazy remote state via JSON-RPC.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs::{create_dir_all, read, write};
use std::hash::{Hash, Hasher};
#[cfg(test)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use alloy_primitives::{Address, B256, U256};
use anyhow::{Context, Result, bail};
use revm::{
    DatabaseRef, bytecode::Bytecode, database_interface::DBErrorMarker, state::AccountInfo,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, trace, warn};

use crate::chain::database::CacheStats;

/// Thin newtype around `anyhow::Error` so we can implement `DBErrorMarker`.
#[derive(Debug)]
pub struct ForkError(pub anyhow::Error);

impl std::fmt::Display for ForkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ForkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl From<anyhow::Error> for ForkError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}

impl DBErrorMarker for ForkError {}

/// The remote + cached backend that satisfies `DatabaseRef`.
///
/// `ForkBackend` is cheaply cloneable (all state lives behind `Arc`).
/// It is intended to be wrapped by `CacheDB`.
#[derive(Clone, Debug)]
pub struct ForkBackend {
    inner: Arc<ForkBackendInner>,
}

#[derive(Debug)]
struct ForkBackendInner {
    /// The RPC client shared with all clones of this backend.
    rpc: Arc<dyn crate::rpc::RpcClient>,
    block_number: u64,
    /// Shared memory cache: account info keyed by address.
    account_cache: RwLock<HashMap<Address, CachedAccount>>,
    /// Shared memory cache: storage slots keyed by (address, slot).
    slot_cache: RwLock<HashMap<(Address, U256), U256>>,
    /// Shared memory cache: block hashes keyed by number.
    block_hash_cache: RwLock<HashMap<u64, B256>>,
    /// Code cache keyed by code hash.
    code_cache: RwLock<HashMap<B256, Bytecode>>,
    /// Path to the persistent cache file for this fork.
    cache_file: PathBuf,
    /// Total cache hits across all cache types.
    cache_hits: AtomicU64,
    /// Total cache misses that triggered an RPC fetch.
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

impl ForkBackend {
    /// Create a new fork backend.
    ///
    /// `project_root` is used to derive the disk cache directory:
    /// `{project_root}/raptor/cache/<hash>/<block>.json`
    pub fn new(
        rpc: Arc<dyn crate::rpc::RpcClient>,
        block_number: u64,
        project_root: &Path,
    ) -> Result<Self> {
        let rpc_url = rpc.cache_key();
        let cache_file = cache_path(project_root, &rpc_url, block_number);

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
            inner: Arc::new(ForkBackendInner {
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
}

// --- Private helpers that return anyhow::Error ---

impl ForkBackend {
    #[instrument(skip(self), fields(%address))]
    fn basic_ref_impl(&self, address: Address) -> Result<Option<AccountInfo>> {
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
    fn code_by_hash_ref_impl(&self, code_hash: B256) -> Result<Bytecode> {
        {
            let cache = self.inner.code_cache.read().map_err(lock_poisoned)?;
            if let Some(code) = cache.get(&code_hash) {
                trace!(%code_hash, "code_by_hash cache hit");
                self.inner.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(code.clone());
            }
        }
        trace!(%code_hash, "code_by_hash cache miss — no RPC fetch available, returning empty");
        self.inner.cache_misses.fetch_add(1, Ordering::Relaxed);
        warn!(%code_hash, "code_by_hash cache miss — returning empty bytecode (expected if hash originated from basic_ref)");
        Ok(Bytecode::default())
    }

    #[instrument(skip(self), fields(%address, %index))]
    fn storage_ref_impl(&self, address: Address, index: U256) -> Result<U256> {
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
    fn block_hash_ref_impl(&self, number: u64) -> Result<B256> {
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

impl ForkBackend {
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

impl DatabaseRef for ForkBackend {
    type Error = ForkError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.basic_ref_impl(address).map_err(ForkError::from)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.code_by_hash_ref_impl(code_hash)
            .map_err(ForkError::from)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.storage_ref_impl(address, index)
            .map_err(ForkError::from)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.block_hash_ref_impl(number).map_err(ForkError::from)
    }
}

impl ForkBackendInner {
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

/// Convert a poisoned `RwLock` into an `anyhow::Error`.
fn lock_poisoned<T>(_: std::sync::PoisonError<T>) -> anyhow::Error {
    anyhow::Error::msg("lock poisoned")
}

/// Derive the canonical cache path: `{project}/raptor/cache/{hash}/{block}.json`
pub fn cache_path(project_root: &Path, rpc_url: &str, block_number: u64) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    rpc_url.hash(&mut hasher);
    let hash = hasher.finish();

    project_root
        .join("raptor")
        .join("cache")
        .join(format!("{:x}", hash))
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
    use revm::Database;

    #[test]
    fn cache_path_derived_correctly() {
        let root = Path::new("/tmp/proj");
        let path = cache_path(root, "https://example.com/rpc", 123);
        assert!(path.to_string_lossy().contains("raptor/cache/"));
        assert!(path.to_string_lossy().ends_with("/123.json"));
    }

    #[test]
    fn parse_u256_valid() {
        let v = serde_json::Value::String("0x1a2b".into());
        assert_eq!(parse_u256(&v).unwrap(), U256::from(0x1a2bu64));
    }

    #[test]
    fn parse_u256_zero() {
        let v = serde_json::Value::String("0x0".into());
        assert_eq!(parse_u256(&v).unwrap(), U256::ZERO);
    }

    #[test]
    fn parse_u256_invalid() {
        let v = serde_json::Value::String("zz".into());
        assert!(parse_u256(&v).is_none());
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

    #[test]
    fn disk_cache_roundtrip() {
        let mut accounts = HashMap::new();
        accounts.insert(
            Address::ZERO,
            CachedAccount {
                balance: U256::from(100),
                nonce: 1,
                code_hash: B256::ZERO,
                code: None,
            },
        );
        let slots = vec![((Address::ZERO, U256::from(0)), U256::from(42))];

        let cache = DiskCache {
            accounts,
            slots,
            block_hashes: HashMap::new(),
            code: HashMap::new(),
        };

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&serde_json::to_vec_pretty(&cache).unwrap())
            .unwrap();

        let loaded: DiskCache = serde_json::from_slice(&read(tmp.path()).unwrap()).unwrap();
        assert_eq!(loaded.accounts.len(), 1);
        assert_eq!(loaded.slots.len(), 1);
    }

    #[test]
    fn sandbox_database_returns_defaults() {
        let mut db = crate::chain::database::Database::default();
        let info = db.basic(Address::ZERO).unwrap();
        assert_eq!(info, None);
        assert_eq!(db.storage(Address::ZERO, U256::ZERO).unwrap(), U256::ZERO);
        // InMemoryDB returns a non-zero placeholder for block_hash(0); just
        // verify the call does not panic.
        let _ = db.block_hash(0).unwrap();
        assert!(db.code_by_hash(B256::ZERO).unwrap().is_empty());
    }

    #[test]
    fn cache_db_clone_isolates_local_writes() {
        let tmpdir = tempfile::tempdir().unwrap();
        let rpc_url = "http://localhost:99999";
        let block = 1u64;

        // Seed disk cache with a remote slot value.
        let slots = vec![((Address::ZERO, U256::from(42)), U256::from(100))];
        let disk = DiskCache {
            accounts: HashMap::new(),
            slots,
            block_hashes: HashMap::new(),
            code: HashMap::new(),
        };
        let cache_file = cache_path(tmpdir.path(), rpc_url, block);
        if let Some(parent) = cache_file.parent() {
            create_dir_all(parent).unwrap();
        }
        write(&cache_file, serde_json::to_vec_pretty(&disk).unwrap()).unwrap();

        let urls: Vec<String> = vec![rpc_url.into()];
        let rpc = Arc::new(
            crate::rpc::Rpc::with_urls(&urls)
                .with_pool_size(1)
                .build()
                .unwrap(),
        );
        let backend = ForkBackend::new(rpc, block, tmpdir.path()).unwrap();

        // Wrap in CacheDB and insert an account so storage insertion works.
        let mut db = revm::database::CacheDB::new(backend.clone());
        db.insert_account_info(Address::ZERO, AccountInfo::default());

        // Write a local slot value.
        db.insert_account_storage(Address::ZERO, U256::from(42), U256::from(999))
            .unwrap();

        // Clone the CacheDB.
        let db_clone = db.clone();

        // The clone must see the local write, not the remote cached value.
        let clone_val = db_clone.storage_ref(Address::ZERO, U256::from(42)).unwrap();
        assert_eq!(clone_val, U256::from(999), "clone should see local write");

        // The underlying backend must still hold the original remote value.
        let remote_val = backend.storage_ref(Address::ZERO, U256::from(42)).unwrap();
        assert_eq!(
            remote_val,
            U256::from(100),
            "backend should still hold cached remote value"
        );
    }

    #[test]
    fn cache_only_no_network_requests() {
        let tmpdir = tempfile::tempdir().unwrap();
        let rpc_url = "http://localhost:99999"; // unreachable on purpose
        let block = 1u64;

        // Pre-seed disk cache with an account and slot.
        let mut accounts = HashMap::new();
        accounts.insert(
            Address::ZERO,
            CachedAccount {
                balance: U256::from(123),
                nonce: 1,
                code_hash: B256::ZERO,
                code: None,
            },
        );
        let slots = vec![((Address::ZERO, U256::from(0)), U256::from(42))];

        let disk = DiskCache {
            accounts,
            slots,
            block_hashes: HashMap::new(),
            code: HashMap::new(),
        };
        let cache_file = cache_path(tmpdir.path(), rpc_url, block);
        if let Some(parent) = cache_file.parent() {
            create_dir_all(parent).unwrap();
        }
        write(&cache_file, serde_json::to_vec_pretty(&disk).unwrap()).unwrap();

        let urls: Vec<String> = vec![rpc_url.into()];
        let rpc = Arc::new(
            crate::rpc::Rpc::with_urls(&urls)
                .with_pool_size(1)
                .build()
                .unwrap(),
        );
        let backend = ForkBackend::new(rpc, block, tmpdir.path()).unwrap();

        // All queries should be satisfied from cache, never hitting the network.
        let info = backend.basic_ref(Address::ZERO).unwrap().unwrap();
        assert_eq!(info.balance, U256::from(123));

        let slot = backend.storage_ref(Address::ZERO, U256::from(0)).unwrap();
        assert_eq!(slot, U256::from(42));
    }

    #[test]
    #[ignore = "requires network: set RAPTOR_FORK_RPC_URL and run with `cargo test -- --ignored`"]
    fn fork_usdc_balance_integration() {
        let rpc_url = std::env::var("RAPTOR_FORK_RPC_URL")
            .unwrap_or_else(|_| "https://eth.llamarpc.com".into());

        let rpc = Arc::new(
            crate::rpc::Rpc::with_urls(&[rpc_url])
                .with_pool_size(1)
                .build()
                .unwrap(),
        );
        let env = crate::chain::environment::Environment::fork(
            rpc,
            25_121_437,
            Path::new("fixtures/forks"),
        );

        let mut config = crate::campaign::CampaignConfig::default();
        config.threads = 1;
        config.max_runs = 100;
        config.timeout_secs = Some(30);

        let artifact = crate::contract::ContractBuilder::for_project(Path::new("fixtures/forks"))
            .with_target_path(Path::new("test/ForkTarget.sol"))
            .build()
            .unwrap();
        let vm = crate::vm::Vm::new(crate::vm::VmConfig::default());
        let chain = crate::chain::Chain::for_artifact(&artifact)
            .with_project(Path::new("fixtures/forks"))
            .with_vm(vm)
            .with_deploy_value(revm::primitives::U256::ZERO)
            .with_deployer(crate::chain::init::DEFAULT_DEPLOYER)
            .with_environment(env)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let campaign = crate::campaign::CampaignBuilder::new()
            .with_project(Path::new("fixtures/forks"))
            .with_artifact(artifact)
            .with_chain(chain)
            .with_config(config)
            .with_fuzzer(crate::fuzzer::DefaultFuzzerFactory)
            .build()
            .unwrap();
        let result = campaign.run().unwrap();

        assert!(
            result.failures.is_empty(),
            "fork campaign should pass all invariants: {:?}",
            result.failures
        );
    }
}
