//! Configuration for [`Chain`](super::Chain) execution behavior.

use std::collections::HashMap;
use std::path::Path;

use revm::primitives::Bytes;

use crate::evm::cheatcode;
use crate::evm::forkdb;

/// Campaign-level configuration that controls chain behaviour.
#[derive(Debug, Clone)]
pub struct Config {
    /// Cheatcode inspector configuration (`vm.ffi`, project root, etc.).
    pub cheatcode: cheatcode::Config,
    /// Enable trace collection.
    pub trace: bool,
    /// Enable coverage collection.
    pub coverage: bool,
    /// Fork configuration; when `Some` the chain is forked from a remote
    /// RPC node instead of starting as an empty sandbox.
    pub fork: Option<forkdb::Config>,
}

impl Config {
    /// Create a new config with the given project root.
    pub fn new(project_root: impl AsRef<Path>) -> Self {
        Self {
            cheatcode: cheatcode::Config::new(project_root),
            trace: false,
            coverage: false,
            fork: None,
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
    pub fn fork(mut self, config: forkdb::Config) -> Self {
        self.fork = Some(config);
        self
    }

    /// Seed compiled contract initcode so `vm.getCode` can resolve artifact
    /// names. Optional; if omitted, `vm.getCode` calls will revert.
    pub fn with_compiled_contracts(mut self, contracts: HashMap<String, Bytes>) -> Self {
        self.cheatcode = self.cheatcode.with_compiled_contracts(contracts);
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_default())
    }
}
