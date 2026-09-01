//! User-facing configuration for the cheatcode inspector.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::evm::forkdb::{ForkDBConfig, Transport};

/// User-facing configuration for the cheatcode inspector.
#[derive(Debug, Clone)]
pub struct CheatcodeConfig {
    /// Enable `vm.ffi` (allows arbitrary host command execution).
    pub ffi: bool,
    /// Foundry project root used by `vm.ffi` to resolve relative paths.
    pub project_root: PathBuf,
    /// Default RPC settings for `vm.fork`.
    pub fork_defaults: ForkDBConfig,
    /// Optional transport override for tests.
    pub transport: Option<Arc<dyn Transport>>,
}

impl CheatcodeConfig {
    /// Create a new config with the given project root.
    pub fn new(project_root: impl AsRef<Path>) -> Self {
        Self {
            ffi: false,
            project_root: project_root.as_ref().to_path_buf(),
            fork_defaults: ForkDBConfig::new(""),
            transport: None,
        }
    }

    /// Enable or disable FFI.
    pub fn ffi(mut self, enabled: bool) -> Self {
        self.ffi = enabled;
        self
    }

    /// Set the project root.
    pub fn with_project_root(mut self, path: impl AsRef<Path>) -> Self {
        self.project_root = path.as_ref().to_path_buf();
        self
    }

    /// Enable or disable FFI (legacy alias for `ffi`).
    pub fn with_ffi(mut self, enabled: bool) -> Self {
        self.ffi = enabled;
        self
    }

    /// Set default RPC settings used by `vm.fork`.
    pub fn with_fork_defaults(mut self, defaults: ForkDBConfig) -> Self {
        self.fork_defaults = defaults;
        self
    }

    /// Inject a custom transport (used by tests with [`crate::evm::MockTransport`]).
    pub fn with_transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = Some(transport);
        self
    }
}

impl Default for CheatcodeConfig {
    fn default() -> Self {
        Self {
            ffi: false,
            project_root: PathBuf::new(),
            fork_defaults: ForkDBConfig::new(""),
            transport: None,
        }
    }
}
