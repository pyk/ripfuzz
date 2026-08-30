//! Configuration for `ripfuzz`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

fn default_out() -> PathBuf {
    PathBuf::from(".ripfuzz/out")
}

/// Configuration loaded from `ripfuzz.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Solc version to use for harness compilation.
    pub solc: String,

    /// Output directory for solc compilation artifacts.
    #[serde(default = "default_out")]
    pub out: PathBuf,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read config `{}`", path.display()))?;
        Self::parse(&content)
    }

    pub fn parse(content: &str) -> Result<Self> {
        toml::from_str(content).context("failed to parse config")
    }
}
