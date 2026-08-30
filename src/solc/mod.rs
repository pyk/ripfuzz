//! Solidity compiler management for `ripfuzz`.
//!
//! Handles downloading and verifying `solc` static binaries from
//! `https://binaries.soliditylang.org` and exposing a builder API for
//! compilation.
//!
//! ```rust
//! use ripfuzz::solc::Solc;
//!
//! let solc = Solc::new().with_version("0.8.28").with_target("src/MyHarness.sol");
//! // solc.compile()?;
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use tracing::info;

pub use installer::SolcInstaller;

pub mod installer;

/// Solidity compiler builder.
#[derive(Clone, Debug, Default)]
pub struct Solc {
    version: Option<String>,
    target: Option<PathBuf>,
}

impl Solc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_target(mut self, target: impl AsRef<Path>) -> Self {
        self.target = Some(target.as_ref().to_path_buf());
        self
    }

    pub fn compile(self) -> Result<()> {
        let version = self
            .version
            .as_deref()
            .context("solc version not set, call Solc::new().with_version(..)")?;
        let target = self
            .target
            .as_deref()
            .context("solc target not set, call Solc::new().with_target(..)")?;

        ensure!(
            target.is_file(),
            "harness file `{}` not found",
            target.display()
        );

        let installer = SolcInstaller::new(version);
        installer.ensure_installed()?;

        info!(version = %version, target = %target.display(), "compiling harness");

        Ok(())
    }
}
