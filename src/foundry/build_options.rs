//! Build options for compiling a Foundry project.

/// Configuration for a `forge build` invocation.
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    force: bool,
}

impl BuildOptions {
    /// Create a new [`BuildOptions`] with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Force recompilation, skipping the cache.
    pub fn force(mut self, force: bool) -> Self {
        self.force = force;
        self
    }

    pub(crate) fn is_force(&self) -> bool {
        self.force
    }
}
