//! EVM world state. Either an empty sandbox or a forked world.

use std::collections::HashMap;

use alloy_primitives::{Address, U256};
use anyhow::Result;
use revm::database::{CacheDB, InMemoryDB};
use revm::state::AccountInfo;

use crate::chain::forkdb::{ForkDB, ForkError};

/// EVM world state. Either an empty sandbox or a forked world.
#[derive(Clone, Debug)]
pub enum Database {
    Sandbox(InMemoryDB),
    Fork(CacheDB<ForkDB>),
}

impl Default for Database {
    fn default() -> Self {
        Self::Sandbox(InMemoryDB::default())
    }
}

impl Database {
    pub fn cache_stats(&self) -> Option<CacheStats> {
        match self {
            Self::Sandbox(_) => None,
            Self::Fork(db) => Some(db.db.cache_stats()),
        }
    }

    pub fn flush_cache(&self) -> Result<()> {
        match self {
            Self::Sandbox(_) => Ok(()),
            Self::Fork(db) => db.db.flush_cache(),
        }
    }

    pub fn insert_account_info(&mut self, address: Address, info: AccountInfo) {
        match self {
            Self::Sandbox(db) => db.insert_account_info(address, info),
            Self::Fork(db) => db.insert_account_info(address, info),
        }
    }

    pub fn insert_account_storage(
        &mut self,
        address: Address,
        slot: U256,
        value: U256,
    ) -> Result<(), DatabaseError> {
        match self {
            Self::Sandbox(db) => db
                .insert_account_storage(address, slot, value)
                .map_err(|e| e.into()),
            Self::Fork(db) => db
                .insert_account_storage(address, slot, value)
                .map_err(|e| e.into()),
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

    fn commit_iter(&mut self, changes: &mut dyn Iterator<Item = (Address, revm::state::Account)>) {
        match self {
            Self::Sandbox(db) => db.commit_iter(changes),
            Self::Fork(db) => db.commit_iter(changes),
        }
    }
}

impl revm::Database for Database {
    type Error = DatabaseError;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self {
            Self::Sandbox(db) => db.basic(address).map_err(|e| e.into()),
            Self::Fork(db) => db.basic(address).map_err(|e| e.into()),
        }
    }

    fn code_by_hash(
        &mut self,
        code_hash: revm::primitives::FixedBytes<32>,
    ) -> Result<revm::bytecode::Bytecode, Self::Error> {
        match self {
            Self::Sandbox(db) => db.code_by_hash(code_hash).map_err(|e| e.into()),
            Self::Fork(db) => db.code_by_hash(code_hash).map_err(|e| e.into()),
        }
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        match self {
            Self::Sandbox(db) => db.storage(address, index).map_err(|e| e.into()),
            Self::Fork(db) => db.storage(address, index).map_err(|e| e.into()),
        }
    }

    fn block_hash(&mut self, number: u64) -> Result<revm::primitives::FixedBytes<32>, Self::Error> {
        match self {
            Self::Sandbox(db) => db.block_hash(number).map_err(|e| e.into()),
            Self::Fork(db) => db.block_hash(number).map_err(|e| e.into()),
        }
    }
}

/// Snapshot of fork cache performance.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

/// Unified error type for the [`Database`] enum.
#[derive(Debug)]
pub enum DatabaseError {
    Fork(ForkError),
}

impl revm::database_interface::DBErrorMarker for DatabaseError {}

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
            Self::Fork(e) => e.source(),
        }
    }
}

impl From<ForkError> for DatabaseError {
    fn from(e: ForkError) -> Self {
        Self::Fork(e)
    }
}

impl From<std::convert::Infallible> for DatabaseError {
    fn from(e: std::convert::Infallible) -> Self {
        match e {}
    }
}
