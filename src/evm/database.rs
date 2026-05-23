//! Unified EVM database: local sandbox and fork backends.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use alloy_primitives::{Address, B256, U256};
use revm::{
    DatabaseCommit, DatabaseRef, bytecode::Bytecode, database::CacheDB,
    database_interface::DBErrorMarker, state::AccountInfo,
};

use crate::rpc_v2::Client;

// ---------------------------------------------------------------------------
// LocalDB
// ---------------------------------------------------------------------------

/// Wrapper around `revm::EmptyDB` that returns `Some(AccountInfo::default())`
/// for every address so that `CacheDB` never marks an account as
/// `AccountState::NotExisting`.
///
/// In revm, `CacheDB` distinguishes between "non-existing" (`None`) and
/// "empty" (`Some(AccountInfo::default())`). If an account is marked as
/// `NotExisting`, state transitions differ when the account is later created
/// (e.g. via `deal` or `etch`). A sandbox fuzzer has no state trie, so every
/// address should be treated as empty rather than non-existing.
///
/// Foundry uses the same trick: see `foundry-evm-core::backend::EmptyDBWrapper`.
/// Local sandbox database backend.
///
/// Returns `Some(AccountInfo::default())` for every address so that `CacheDB`
/// never marks an account as `AccountState::NotExisting`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalDB(revm::database::EmptyDBTyped<std::convert::Infallible>);

impl DatabaseRef for LocalDB {
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

// ---------------------------------------------------------------------------
// ForkDB
// ---------------------------------------------------------------------------

/// Thin newtype around `anyhow::Error` so we can implement `DBErrorMarker`.
#[derive(Debug)]
pub struct ForkDBError(anyhow::Error);

impl std::fmt::Display for ForkDBError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ForkDBError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl DBErrorMarker for ForkDBError {}

impl From<anyhow::Error> for ForkDBError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}

/// Remote backend that satisfies `DatabaseRef`.
///
/// All state caching is delegated to the RPC layer; this struct only maps
/// revm database operations to typed RPC calls.
#[derive(Clone, Debug)]
pub struct ForkDB {
    client: Arc<Client>,
    block_number: u64,
    /// Caches bytecode by code hash. RwLock chosen because `code_by_hash_ref`
    /// is read-heavy while writes only happen on cache misses during
    /// `basic_ref`.
    contracts: Arc<RwLock<HashMap<B256, Bytecode>>>,
}

impl ForkDB {
    pub fn new(client: Arc<Client>, block_number: u64) -> Self {
        Self {
            client,
            block_number,
            contracts: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl DatabaseRef for ForkDB {
    type Error = ForkDBError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let (balance, nonce, code) = self
            .client
            .get_account(address, self.block_number)
            .map_err(ForkDBError::from)?;
        let bytecode = if code.is_empty() {
            Bytecode::default()
        } else {
            Bytecode::new_raw(code)
        };
        let code_hash = bytecode.hash_slow();
        if !bytecode.is_empty() {
            self.contracts
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(code_hash, bytecode.clone());
        }
        Ok(Some(AccountInfo {
            balance,
            nonce,
            code_hash,
            code: Some(bytecode),
            account_id: None,
        }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if code_hash == revm::primitives::KECCAK_EMPTY || code_hash.is_zero() {
            return Ok(Bytecode::default());
        }
        match self
            .contracts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&code_hash)
        {
            Some(code) => Ok(code.clone()),
            None => Err(ForkDBError::from(anyhow::anyhow!(
                "code hash {} not found in fork database",
                code_hash
            ))),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.client
            .get_storage_at(address, index, self.block_number)
            .map_err(ForkDBError::from)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let block = self
            .client
            .get_block_by_number(number)
            .map_err(ForkDBError::from)?;
        Ok(block.hash.unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// ForkConfig
// ---------------------------------------------------------------------------

/// Configuration for a forked chain.
#[derive(Debug, Clone)]
pub struct ForkConfig {
    pub client: Arc<Client>,
    pub block_number: u64,
}

// ---------------------------------------------------------------------------
// DatabaseError
// ---------------------------------------------------------------------------

/// Unified database error.
#[derive(Debug)]
pub enum DatabaseError {
    Fork(ForkDBError),
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

impl From<ForkDBError> for DatabaseError {
    fn from(e: ForkDBError) -> Self {
        Self::Fork(e)
    }
}

impl From<std::convert::Infallible> for DatabaseError {
    fn from(e: std::convert::Infallible) -> Self {
        match e {}
    }
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// Unified EVM database.
#[derive(Clone, Debug)]
pub enum Database {
    Local(CacheDB<LocalDB>),
    Fork(CacheDB<ForkDB>),
}

impl Database {
    /// Insert or override account info.
    pub fn insert_account_info(&mut self, address: Address, info: AccountInfo) {
        match self {
            Self::Local(db) => db.insert_account_info(address, info),
            Self::Fork(db) => db.insert_account_info(address, info),
        }
    }
}

impl revm::Database for Database {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Local(db) => db.basic(address).map_err(|e| match e {}),
            Self::Fork(db) => db.basic(address).map_err(DatabaseError::from),
        }
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self {
            Self::Local(db) => db.code_by_hash(code_hash).map_err(|e| match e {}),
            Self::Fork(db) => db.code_by_hash(code_hash).map_err(DatabaseError::from),
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Local(db) => db.storage(address, index).map_err(|e| match e {}),
            Self::Fork(db) => db.storage(address, index).map_err(DatabaseError::from),
        }
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        match self {
            Self::Local(db) => db.block_hash(number).map_err(|e| match e {}),
            Self::Fork(db) => db.block_hash(number).map_err(DatabaseError::from),
        }
    }
}

impl DatabaseCommit for Database {
    fn commit(
        &mut self,
        changes: HashMap<Address, revm::state::Account, revm::primitives::map::FbBuildHasher<20>>,
    ) {
        match self {
            Self::Local(db) => db.commit(changes),
            Self::Fork(db) => db.commit(changes),
        }
    }
}

impl DatabaseRef for Database {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Local(db) => db.basic_ref(address).map_err(|e| match e {}),
            Self::Fork(db) => db.basic_ref(address).map_err(DatabaseError::from),
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self {
            Self::Local(db) => db.code_by_hash_ref(code_hash).map_err(|e| match e {}),
            Self::Fork(db) => db.code_by_hash_ref(code_hash).map_err(DatabaseError::from),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Local(db) => db.storage_ref(address, index).map_err(|e| match e {}),
            Self::Fork(db) => db.storage_ref(address, index).map_err(DatabaseError::from),
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        match self {
            Self::Local(db) => db.block_hash_ref(number).map_err(|e| match e {}),
            Self::Fork(db) => db.block_hash_ref(number).map_err(DatabaseError::from),
        }
    }
}
