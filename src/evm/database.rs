//! Unified EVM database: empty chain and fork chain backends.

use std::collections::HashMap;

use alloy_primitives::{Address, B256, U256};
use revm::{
    DatabaseCommit, DatabaseRef, bytecode::Bytecode, database::CacheDB,
    database_interface::DBErrorMarker, state::AccountInfo,
};

use crate::evm::forkdb;
use crate::evm::forkdb::ForkDB;

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

/// Unified EVM database.
#[derive(Clone, Debug)]
pub enum Database {
    Empty(CacheDB<EmptyDB>),
    Fork(CacheDB<ForkDB>),
}

impl Database {
    /// Insert or override account info.
    pub fn insert_account_info(&mut self, address: Address, info: AccountInfo) {
        match self {
            Self::Empty(db) => db.insert_account_info(address, info),
            Self::Fork(db) => db.insert_account_info(address, info),
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
            Self::Fork(db) => db
                .insert_account_storage(address, slot, value)
                .map_err(DatabaseError::from),
        }
    }
}

impl revm::Database for Database {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Empty(db) => db.basic(address).map_err(|e| match e {}),
            Self::Fork(db) => db.basic(address).map_err(DatabaseError::from),
        }
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self {
            Self::Empty(db) => db.code_by_hash(code_hash).map_err(|e| match e {}),
            Self::Fork(db) => db.code_by_hash(code_hash).map_err(DatabaseError::from),
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Empty(db) => db.storage(address, index).map_err(|e| match e {}),
            Self::Fork(db) => db.storage(address, index).map_err(DatabaseError::from),
        }
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        match self {
            Self::Empty(db) => db.block_hash(number).map_err(|e| match e {}),
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
            Self::Empty(db) => db.commit(changes),
            Self::Fork(db) => db.commit(changes),
        }
    }
}

impl DatabaseRef for Database {
    type Error = DatabaseError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Empty(db) => db.basic_ref(address).map_err(|e| match e {}),
            Self::Fork(db) => db.basic_ref(address).map_err(DatabaseError::from),
        }
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        match self {
            Self::Empty(db) => db.code_by_hash_ref(code_hash).map_err(|e| match e {}),
            Self::Fork(db) => db.code_by_hash_ref(code_hash).map_err(DatabaseError::from),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Empty(db) => db.storage_ref(address, index).map_err(|e| match e {}),
            Self::Fork(db) => db.storage_ref(address, index).map_err(DatabaseError::from),
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        match self {
            Self::Empty(db) => db.block_hash_ref(number).map_err(|e| match e {}),
            Self::Fork(db) => db.block_hash_ref(number).map_err(DatabaseError::from),
        }
    }
}
