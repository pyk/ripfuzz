//! Unified database and fork backend for chain_v2.

use std::collections::HashMap;
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use anyhow::Result;
use revm::{
    DatabaseRef,
    bytecode::Bytecode,
    database::{CacheDB, InMemoryDB},
    database_interface::DBErrorMarker,
    state::AccountInfo,
};
use tracing::instrument;

use crate::rpc_v2::Client;

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

/// Remote backend that satisfies `DatabaseRef`.
///
/// All state caching is delegated to the RPC layer; this struct only maps
/// revm database operations to typed RPC calls.
#[derive(Clone, Debug)]
pub struct ForkDb {
    rpc: Arc<Client>,
    block_number: u64,
}

impl ForkDb {
    pub fn new(rpc: Arc<Client>, block_number: u64) -> Self {
        Self { rpc, block_number }
    }
}

impl DatabaseRef for ForkDb {
    type Error = ForkDbError;

    #[instrument(skip(self), fields(%address))]
    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let balance = self.rpc.get_balance(address, self.block_number)?;
        let nonce = self.rpc.get_transaction_count(address, self.block_number)?;
        let code = self.rpc.get_code(address, self.block_number)?;
        let bytecode = if code.is_empty() {
            Bytecode::default()
        } else {
            Bytecode::new_raw(code)
        };
        let code_hash = bytecode.hash_slow();

        Ok(Some(AccountInfo {
            balance,
            nonce,
            code_hash,
            code: Some(bytecode),
            account_id: None,
        }))
    }

    #[instrument(skip(self), fields(%_code_hash))]
    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        // Standard JSON-RPC does not expose a code-by-hash endpoint.
        // The outer CacheDB caches code loaded via basic_ref, so this
        // path is rarely exercised in practice.
        Ok(Bytecode::default())
    }

    #[instrument(skip(self), fields(%address, %index))]
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.rpc
            .get_storage_at(address, index, self.block_number)
            .map_err(Into::into)
    }

    #[instrument(skip(self), fields(number))]
    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let block = self.rpc.get_block_by_number(number)?;
        Ok(block.hash.unwrap_or_default())
    }
}
