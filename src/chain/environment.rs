//! Execution environment: the researcher's choice of sandbox or fork.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;

use crate::chain::database::Database;

/// The researcher's choice of execution world.
#[derive(Debug)]
pub enum Environment {
    /// Empty sandbox. No RPC. No remote state.
    Sandbox,
    /// Fork from a live network at a specific block.
    Fork {
        rpc: Arc<dyn crate::rpc::RpcClient>,
        block_number: u64,
        project_root: PathBuf,
    },
}

impl Environment {
    pub fn sandbox() -> Self {
        Self::Sandbox
    }

    pub fn fork(
        rpc: Arc<dyn crate::rpc::RpcClient>,
        block_number: u64,
        project_root: &Path,
    ) -> Self {
        Self::Fork {
            rpc,
            block_number,
            project_root: project_root.to_path_buf(),
        }
    }

    /// Build a [`Database`] from this environment.
    pub fn create_database(&self) -> anyhow::Result<Database> {
        match self {
            Environment::Sandbox => Ok(Database::Sandbox(revm::database::InMemoryDB::default())),
            Environment::Fork {
                rpc,
                block_number,
                project_root,
            } => {
                let backend = crate::chain::fork::ForkBackend::new(
                    Arc::clone(rpc),
                    *block_number,
                    project_root,
                )
                .context("fork initialization failed")?;
                Ok(Database::Fork(revm::database::CacheDB::new(backend)))
            }
        }
    }
}
