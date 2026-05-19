//! Execution environment.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use revm::database::{CacheDB, InMemoryDB};

use crate::chain::Database;
use crate::chain::ForkDB;
use crate::rpc::RpcClient;

/// Fuzzing environment
#[derive(Debug)]
pub enum Environment {
    /// Empty sandbox. No RPC. No remote state.
    Sandbox,
    /// Fork from a live network at a specific block.
    Fork {
        /// We use `Arc` here because `rpc` is shared with `ForkDB` and all
        /// its clones, so every database operation sees the same connection pool
        /// and cache.
        rpc: Arc<dyn RpcClient>,
        block_number: u64,
        project_path: PathBuf,
    },
}

impl Environment {
    pub fn sandbox() -> Self {
        Self::Sandbox
    }

    pub fn fork(
        rpc: Arc<dyn RpcClient>,
        block_number: u64,
        project_path: impl AsRef<Path>,
    ) -> Self {
        Self::Fork {
            rpc,
            block_number,
            project_path: project_path.as_ref().to_path_buf(),
        }
    }

    /// Build a [`Database`] from this environment.
    pub fn create_database(&self) -> Result<Database> {
        match self {
            Environment::Sandbox => Ok(Database::Sandbox(InMemoryDB::default())),
            Environment::Fork {
                rpc,
                block_number,
                project_path,
            } => {
                let db = ForkDB::new(Arc::clone(rpc), *block_number, project_path)
                    .context("fork initialization failed")?;
                Ok(Database::Fork(CacheDB::new(db)))
            }
        }
    }
}
