//! Configuration for [`Chain`](super::Chain) execution behavior.

use std::collections::HashMap;
use std::path::Path;

use revm::primitives::Bytes;

use crate::evm::cheatcode::CheatcodeConfig;
use crate::evm::forkdb::ForkDBConfig;

/// Campaign-level configuration that controls chain behaviour.
#[derive(Debug, Clone)]
pub struct ChainConfig {
    cheatcode: CheatcodeConfig,
    trace: bool,
    coverage: bool,
    fork: Option<ForkDBConfig>,
}

impl ChainConfig {
    /// Create a new config with the given project root.
    pub fn new(project_root: impl AsRef<Path>) -> Self {
        Self {
            cheatcode: CheatcodeConfig::new(project_root),
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
    pub fn fork(mut self, config: ForkDBConfig) -> Self {
        self.fork = Some(config);
        self
    }

    /// Seed compiled contract initcode so `vm.getCode` can resolve artifact
    /// names. Optional; if omitted, `vm.getCode` calls will revert.
    pub fn with_compiled_contracts(mut self, contracts: HashMap<String, Bytes>) -> Self {
        self.cheatcode = self.cheatcode.with_compiled_contracts(contracts);
        self
    }

    /// Enable or disable FFI via the cheatcode inspector.
    pub fn ffi(mut self, enabled: bool) -> Self {
        self.cheatcode = self.cheatcode.ffi(enabled);
        self
    }

    /// Enable or disable trace collection on an existing config.
    pub fn set_trace(&mut self, enabled: bool) {
        self.trace = enabled;
    }

    /// Whether trace collection is enabled.
    pub fn trace_enabled(&self) -> bool {
        self.trace
    }

    /// Whether coverage collection is enabled.
    pub fn coverage_enabled(&self) -> bool {
        self.coverage
    }

    /// Fork configuration, if any.
    pub fn fork_config(&self) -> Option<&ForkDBConfig> {
        self.fork.as_ref()
    }

    /// Cheatcode inspector configuration.
    pub fn cheatcode(&self) -> &CheatcodeConfig {
        &self.cheatcode
    }
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_default())
    }
}
