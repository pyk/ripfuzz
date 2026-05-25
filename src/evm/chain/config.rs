//! Configuration for [`Chain`](super::Chain) execution behavior.

use std::path::Path;

/// Campaign-level configuration that controls inspector behavior across
/// all `deploy`, `setup`, and `exec` calls on a [`Chain`].
#[derive(Debug, Clone)]
pub struct Config {
    /// Cheatcode inspector configuration (`vm.ffi`, project root, etc.).
    pub cheatcode: crate::evm::cheatcode::Config,
    /// Enable trace collection.
    pub trace: bool,
    /// Enable coverage collection.
    pub coverage: bool,
}

impl Config {
    /// Create a new config with the given project root.
    pub fn new(project_root: impl AsRef<Path>) -> Self {
        Self {
            cheatcode: crate::evm::cheatcode::Config::new(project_root),
            trace: false,
            coverage: false,
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
}

impl Default for Config {
    fn default() -> Self {
        Self::new(std::env::current_dir().unwrap_or_default())
    }
}
