//! User-facing configuration for the cheatcode inspector.

use std::path::{Path, PathBuf};

/// User-facing configuration for the cheatcode inspector.
#[derive(Debug, Clone)]
pub struct Config {
    /// Enable `vm.ffi` (allows arbitrary host command execution).
    pub ffi: bool,
    /// Foundry project root used by `vm.getCode` and `vm.ffi`.
    pub project_root: PathBuf,
}

impl Config {
    /// Create a new config with the given project root.
    pub fn new(project_root: impl AsRef<Path>) -> Self {
        Self {
            ffi: false,
            project_root: project_root.as_ref().to_path_buf(),
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ffi: false,
            project_root: PathBuf::new(),
        }
    }
}
