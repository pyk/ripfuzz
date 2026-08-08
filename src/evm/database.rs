//! Unified EVM database: empty chain and multi-fork backends.

use std::collections::HashMap;
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use revm::{
    DatabaseCommit, DatabaseRef, bytecode::Bytecode, database::CacheDB,
    database_interface::DBErrorMarker, primitives::KECCAK_EMPTY, state::AccountInfo,
};

use crate::evm::chain::DEFAULT_DEPLOYER;
use crate::evm::cheatcode::VM_ADDRESS;
use crate::evm::forkdb;
use crate::evm::forkdb::{
    ForkDB, ForkDBConfig, SharedBackend, SharedLocalAddressRegistry, Transport, url_hash,
};
use crate::evm::specs;

/// Extension trait for databases that can pre-populate storage slots
/// without hitting the underlying network backend.
pub trait DatabaseExt {
    type Error;
    fn insert_account_storage(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> Result<(), Self::Error>;
}

impl DatabaseExt for Database {
    type Error = DatabaseError;

    fn insert_account_storage(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> Result<(), Self::Error> {
        Database::insert_account_storage(self, address, slot, value)
    }
}

/// No-op implementation for the empty (non-fork) database used in tests.
impl DatabaseExt for revm::database::EmptyDBTyped<std::convert::Infallible> {
    type Error = std::convert::Infallible;

    fn insert_account_storage(
        &mut self,
        _address: Address,
        _slot: U256,
        _value: U256,
    ) -> Result<(), Self::Error> {
        // Empty chain has no remote backend -- nothing to pre-cache.
        Ok(())
    }
}

/// Wrapper around `revm::EmptyDB` that returns `Some(AccountInfo::default())`
/// for every address so that `CacheDB` never marks an account as
/// `AccountState::NotExisting`.
///
/// In revm, `CacheDB` distinguishes between "non-existing" (`None`) and
/// "empty" (`Some(AccountInfo::default())`). If an account is marked as
/// `NotExisting`, state transitions differ when the account is later created
/// (e.g. via `deal` or `etch`). An empty-chain fuzzer has no state trie, so
/// every address should be treated as empty rather than non-existing.
///
/// Foundry uses the same trick: see `foundry-evm-core::backend::EmptyDBWrapper`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmptyDB(revm::database::EmptyDBTyped<std::convert::Infallible>);

impl DatabaseRef for EmptyDB {
    type Error = std::convert::Infallible;

    fn basic_ref(&self, _address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(Some(AccountInfo::default()))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.0.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.0.storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.0.block_hash_ref(number)
    }
}

/// Unified database error.
#[derive(Debug)]
pub enum DatabaseError {
    Fork(forkdb::Error),
    ForkSelect(String),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fork(e) => e.fmt(f),
            Self::ForkSelect(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Fork(e) => Some(e),
            Self::ForkSelect(_) => None,
        }
    }
}

impl DBErrorMarker for DatabaseError {}

impl From<forkdb::Error> for DatabaseError {
    fn from(e: forkdb::Error) -> Self {
        Self::Fork(e)
    }
}

impl From<std::convert::Infallible> for DatabaseError {
    fn from(e: std::convert::Infallible) -> Self {
        match e {}
    }
}

/// Identity of a fork: RPC endpoint + pinned block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ForkKey {
    pub url_hash: u64,
    pub block_number: u64,
}

/// Block / chain environment produced when a fork is created or selected.
#[derive(Debug, Clone)]
pub struct ForkEnv {
    pub chain_id: u64,
    pub block_number: u64,
    pub timestamp: U256,
    pub beneficiary: Address,
    pub difficulty: U256,
    pub prevrandao: Option<B256>,
    pub excess_blob_gas: Option<u64>,
    pub block_hash: Option<B256>,
    pub spec_id: revm::primitives::hardfork::SpecId,
}

/// Cached account info plus storage slots to copy across forks.
type PersistentAccount = (Address, AccountInfo, Vec<(U256, U256)>);

/// Per-fork overlay and metadata.
#[derive(Clone, Debug)]
struct ForkSlot {
    db: CacheDB<ForkDB>,
    env: ForkEnv,
}

/// Multi-fork database with one active fork and independent remote overlays.
#[derive(Clone, Debug)]
pub struct MultiForkDB {
    active: ForkKey,
    forks: HashMap<ForkKey, ForkSlot>,
    /// Shared backends keyed by RPC URL hash so multiple blocks on the same
    /// endpoint reuse the same cache and batcher.
    backends: HashMap<u64, SharedBackend>,
    local_registry: SharedLocalAddressRegistry,
}

/// Unified EVM database.
#[derive(Clone, Debug)]
pub enum Database {
    Empty(CacheDB<EmptyDB>),
    Multi(MultiForkDB),
}

impl Database {
    /// Insert or override account info on the active database.
    pub fn insert_account_info(&mut self, address: Address, info: AccountInfo) {
        match self {
            Self::Empty(db) => db.insert_account_info(address, info),
            Self::Multi(multi) => {
                if let Ok(db) = multi.active_db_mut() {
                    db.insert_account_info(address, info);
                }
            }
        }
    }

    /// Pre-populate a storage slot in the cache without triggering a fork DB
    /// fetch. This is used by the `vm.store` cheatcode to avoid an
    /// unnecessary `eth_getStorageAt` call when writing a value the caller
    /// intends to overwrite.
    pub fn insert_account_storage(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> Result<(), DatabaseError> {
        match self {
            Self::Empty(db) => db
                .insert_account_storage(address, slot, value)
                .map_err(|e| match e {}),
            Self::Multi(multi) => multi
                .active_db_mut()?
                .insert_account_storage(address, slot, value)
                .map_err(DatabaseError::from),
        }
    }

    /// Returns true when `(url, block_number)` is already the active multi-fork.
    ///
    /// Empty (sandbox) databases always return false so the first `vm.fork`
    /// still runs the mid-tx journal commit path when needed.
    pub fn is_active_fork(&self, url: &str, block_number: u64) -> bool {
        match self {
            Self::Empty(_) => false,
            Self::Multi(multi) => {
                let key = ForkKey {
                    url_hash: url_hash(url),
                    block_number,
                };
                multi.active == key && multi.forks.contains_key(&key)
            }
        }
    }

    /// Create or select a fork and make it active.
    ///
    /// Local accounts (deployer, VM, and addresses in `local_registry`) are
    /// copied into the selected fork so harness state survives fork switches.
    pub fn fork(
        &mut self,
        url: &str,
        block_number: u64,
        mut config: ForkDBConfig,
        transport: Option<Arc<dyn Transport>>,
        local_registry: SharedLocalAddressRegistry,
    ) -> Result<ForkEnv, DatabaseError> {
        config.url = url.to_owned();
        config.block_number = block_number;
        let key = ForkKey {
            url_hash: url_hash(url),
            block_number,
        };

        match self {
            Self::Multi(multi) => {
                if multi.active == key && multi.forks.contains_key(&key) {
                    return Ok(multi.forks[&key].env.clone());
                }
                multi.select_or_create(key, config, transport)?;
                Ok(multi.forks[&multi.active].env.clone())
            }
            Self::Empty(empty) => {
                let persistent = collect_cached_accounts(empty);
                let mut multi = MultiForkDB {
                    active: key,
                    forks: HashMap::new(),
                    backends: HashMap::new(),
                    local_registry: local_registry.clone(),
                };
                multi.create_fork(key, config, transport, &persistent)?;
                let env = multi.forks[&key].env.clone();
                *self = Self::Multi(multi);
                Ok(env)
            }
        }
    }
}

impl MultiForkDB {
    fn active_db_mut(&mut self) -> Result<&mut CacheDB<ForkDB>, DatabaseError> {
        self.forks
            .get_mut(&self.active)
            .map(|slot| &mut slot.db)
            .ok_or_else(|| DatabaseError::ForkSelect("active fork missing".into()))
    }

    fn active_db(&self) -> Result<&CacheDB<ForkDB>, DatabaseError> {
        self.forks
            .get(&self.active)
            .map(|slot| &slot.db)
            .ok_or_else(|| DatabaseError::ForkSelect("active fork missing".into()))
    }

    fn select_or_create(
        &mut self,
        key: ForkKey,
        config: ForkDBConfig,
        transport: Option<Arc<dyn Transport>>,
    ) -> Result<(), DatabaseError> {
        if self.forks.contains_key(&key) {
            let persistent = self.collect_persistent_from_active()?;
            let target = self
                .forks
                .get_mut(&key)
                .ok_or_else(|| DatabaseError::ForkSelect("fork missing".into()))?;
            apply_persistent(&mut target.db, &persistent);
            self.active = key;
            return Ok(());
        }
        let persistent = self.collect_persistent_from_active()?;
        self.create_fork(key, config, transport, &persistent)?;
        Ok(())
    }

    fn create_fork(
        &mut self,
        key: ForkKey,
        config: ForkDBConfig,
        transport: Option<Arc<dyn Transport>>,
        persistent: &[PersistentAccount],
    ) -> Result<(), DatabaseError> {
        let transport = transport.clone();
        let backend_config = config.clone();
        let backend = self
            .backends
            .entry(key.url_hash)
            .or_insert_with(|| match transport {
                Some(t) => SharedBackend::new_with_transport(backend_config, TransportClone(t)),
                None => SharedBackend::new(backend_config),
            })
            .clone();

        let (db, env) = build_fork_slot(backend, self.local_registry.clone(), &config)?;
        let mut slot = ForkSlot { db, env };
        apply_persistent(&mut slot.db, persistent);
        self.forks.insert(key, slot);
        self.active = key;
        Ok(())
    }

    fn collect_persistent_from_active(&self) -> Result<Vec<PersistentAccount>, DatabaseError> {
        let active = self.active_db()?;
        let mut out = Vec::new();
        for (addr, acc) in active.cache.accounts.iter() {
            if !is_persistent(*addr, &self.local_registry) {
                continue;
            }
            let mut storage = Vec::new();
            for (k, v) in acc.storage.iter() {
                storage.push((*k, *v));
            }
            // checkrs: allow(clone_in_loops)
            out.push((*addr, acc.info.clone(), storage));
        }
        Ok(out)
    }
}

/// Wrapper so `Arc<dyn Transport>` can be passed where `impl Transport` is needed.
#[derive(Debug, Clone)]
struct TransportClone(Arc<dyn Transport>);

impl Transport for TransportClone {
    fn exec(
        &self,
        url: &str,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value, anyhow::Error> {
        self.0.exec(url, payload)
    }
}

fn is_persistent(addr: Address, registry: &SharedLocalAddressRegistry) -> bool {
    addr == DEFAULT_DEPLOYER || addr == VM_ADDRESS || registry.is_local(addr)
}

fn collect_cached_accounts(db: &CacheDB<EmptyDB>) -> Vec<PersistentAccount> {
    let mut out = Vec::new();
    for (addr, acc) in db.cache.accounts.iter() {
        let mut storage = Vec::new();
        for (k, v) in acc.storage.iter() {
            storage.push((*k, *v));
        }
        // checkrs: allow(clone_in_loops)
        out.push((*addr, acc.info.clone(), storage));
    }
    out
}

fn apply_persistent(db: &mut CacheDB<ForkDB>, persistent: &[PersistentAccount]) {
    for (addr, info, storage) in persistent {
        // checkrs: allow(clone_in_loops)
        db.insert_account_info(*addr, info.clone());
        for (slot, value) in storage {
            let _ = db.insert_account_storage(*addr, *slot, *value);
        }
    }
}

fn build_fork_slot(
    backend: SharedBackend,
    local_registry: SharedLocalAddressRegistry,
    config: &ForkDBConfig,
) -> Result<(CacheDB<ForkDB>, ForkEnv), DatabaseError> {
    let block_number = config.block_number;
    let url_hash = url_hash(&config.url);

    let mut responses = backend
        .fetch_or_wait(&[forkdb::Request::GetChainId { url_hash }])
        .map_err(DatabaseError::from)?;
    let chain_id = responses
        .pop()
        .and_then(|r| match r {
            forkdb::Response::ChainId(v) => Some(v),
            _ => None,
        })
        .ok_or_else(|| DatabaseError::ForkSelect("missing ChainId response".into()))?;

    let mut responses = backend
        .fetch_or_wait(&[forkdb::Request::GetBlockByNumber {
            chain_id,
            block: block_number,
        }])
        .map_err(DatabaseError::from)?;
    let block = responses
        .pop()
        .and_then(|r| match r {
            forkdb::Response::BlockByNumber(b) => Some(b),
            _ => None,
        })
        .ok_or_else(|| DatabaseError::ForkSelect("missing BlockByNumber response".into()))?;

    let returned_number = block.number.to::<u64>();
    if returned_number != block_number {
        return Err(DatabaseError::ForkSelect(format!(
            "RPC returned block {returned_number} but requested block {block_number}"
        )));
    }

    let timestamp = U256::from(block.timestamp);
    let spec_id = specs::get_spec_id(chain_id, block.timestamp.to());
    let fork_db = ForkDB::new(backend, local_registry, block_number, chain_id);
    let mut database = CacheDB::new(fork_db);

    if let Some(hash) = block.hash {
        database
            .cache
            .block_hashes
            .insert(U256::from(block.number), hash);
    }

    // Seed coinbase so gas payment does not trigger an RPC fetch.
    database.insert_account_info(
        block.coinbase,
        AccountInfo {
            balance: U256::MAX,
            nonce: 0,
            code_hash: KECCAK_EMPTY,
            code: None,
            account_id: None,
        },
    );

    let env = ForkEnv {
        chain_id,
        block_number,
        timestamp,
        beneficiary: block.coinbase,
        difficulty: block.difficulty,
        prevrandao: block.prevrandao,
        excess_blob_gas: block.excess_blob_gas.map(|v| v.to::<u64>()),
        block_hash: block.hash,
        spec_id,
    };

    Ok((database, env))
}

impl revm::Database for Database {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Empty(db) => db.basic(address).map_err(|e| match e {}),
            Self::Multi(multi) => multi
                .active_db_mut()?
                .basic(address)
                .map_err(DatabaseError::from),
        }
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self {
            Self::Empty(db) => db.code_by_hash(code_hash).map_err(|e| match e {}),
            Self::Multi(multi) => multi
                .active_db_mut()?
                .code_by_hash(code_hash)
                .map_err(DatabaseError::from),
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Empty(db) => db.storage(address, index).map_err(|e| match e {}),
            Self::Multi(multi) => multi
                .active_db_mut()?
                .storage(address, index)
                .map_err(DatabaseError::from),
        }
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        match self {
            Self::Empty(db) => db.block_hash(number).map_err(|e| match e {}),
            Self::Multi(multi) => multi
                .active_db_mut()?
                .block_hash(number)
                .map_err(DatabaseError::from),
        }
    }
}

impl DatabaseCommit for Database {
    fn commit(
        &mut self,
        changes: HashMap<Address, revm::state::Account, revm::primitives::map::FbBuildHasher<20>>,
    ) {
        match self {
            Self::Empty(db) => db.commit(changes),
            Self::Multi(multi) => {
                if let Ok(db) = multi.active_db_mut() {
                    db.commit(changes);
                }
            }
        }
    }
}

impl DatabaseRef for Database {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Empty(db) => db.basic_ref(address).map_err(|e| match e {}),
            Self::Multi(multi) => multi
                .active_db()?
                .basic_ref(address)
                .map_err(DatabaseError::from),
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self {
            Self::Empty(db) => db.code_by_hash_ref(code_hash).map_err(|e| match e {}),
            Self::Multi(multi) => multi
                .active_db()?
                .code_by_hash_ref(code_hash)
                .map_err(DatabaseError::from),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Empty(db) => db.storage_ref(address, index).map_err(|e| match e {}),
            Self::Multi(multi) => multi
                .active_db()?
                .storage_ref(address, index)
                .map_err(DatabaseError::from),
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        match self {
            Self::Empty(db) => db.block_hash_ref(number).map_err(|e| match e {}),
            Self::Multi(multi) => multi
                .active_db()?
                .block_hash_ref(number)
                .map_err(DatabaseError::from),
        }
    }
}
