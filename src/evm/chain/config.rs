//! Configuration for [`Chain`](super::Chain) execution behavior.

use std::path::Path;

/// Campaign-level configuration that controls chain behaviour.
#[derive(Debug, Clone)]
pub struct Config {
    /// Cheatcode inspector configuration (`vm.ffi`, project root, etc.).
    pub cheatcode: crate::evm::cheatcode::Config,
    /// Enable trace collection.
    pub trace: bool,
    /// Enable coverage collection.
    pub coverage: bool,
    /// Fork configuration; when `Some` the chain is forked from a remote
    /// RPC node instead of starting as an empty sandbox.
    pub fork: Option<crate::evm::forkdb::Config>,
    /// Block number to pin the fork to. Required when `fork` is `Some`.
    pub fork_block_number: Option<u64>,
}

impl Config {
    /// Create a new config with the given project root.
    pub fn new(project_root: impl AsRef<Path>) -> Self {
        Self {
            cheatcode: crate::evm::cheatcode::Config::new(project_root),
            trace: false,
            coverage: false,
            fork: None,
            fork_block_number: None,
        }
    }

    /// Enable or disable trace collection.
    pub fn trace(mut self, enabled: bool) -> Self {
        self.trace = enabled;
        self
    }

    /// Enable or disable coverage collection.
    pub fn coverage(mut self, enabled: bool) -> Self {
        self.coverage = enabled;
        self
    }

    /// Set the fork configuration.
    pub fn fork(mut self, config: crate::evm::forkdb::Config) -> Self {
        self.fork = Some(config);
        self
    }

    /// Set the fork block number.
    pub fn fork_block_number(mut self, block_number: u64) -> Self {
        self.fork_block_number = Some(block_number);
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_default())
    }
}
