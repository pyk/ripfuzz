//! User-facing configuration for the VM contract.

use std::path::{Path, PathBuf};

/// User-facing configuration for the VM contract.
#[derive(Debug, Clone)]
pub struct VmConfig {
    /// Enable `vm.ffi` (allows arbitrary host command execution).
    pub ffi: bool,
    /// Foundry project root used by `vm.getCode` and `vm.ffi`.
    pub project_root: PathBuf,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            ffi: false,
            project_root: PathBuf::new(),
        }
    }
}

impl VmConfig {
    pub fn with_ffi(mut self, enabled: bool) -> Self {
        self.ffi = enabled;
        self
    }

    pub fn with_project_root(mut self, path: impl AsRef<Path>) -> Self {
        self.project_root = path.as_ref().to_path_buf();
        self
    }
}
