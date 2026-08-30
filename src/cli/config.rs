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

    /// Root used to resolve the config path. Not part of the TOML file.
    #[serde(skip)]
    root: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    pub fn new() -> Self {
        Self {
            solc: String::new(),
            out: default_out(),
            root: PathBuf::from("."),
        }
    }

    pub fn with_root(mut self, root: impl AsRef<Path>) -> Self {
        self.root = root.as_ref().to_path_buf();
        self
    }

    pub fn load(&self, path: impl AsRef<Path>) -> Result<Self> {
        let path = self.join(path.as_ref());
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config `{}`", path.display()))?;
        let mut config = Self::parse(&content)?;
        config.root = self.root.clone();
        Ok(config)
    }

    pub fn parse(content: &str) -> Result<Self> {
        toml::from_str(content).context("failed to parse config")
    }

    fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn load_resolves_relative_path_against_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ripfuzz.toml"), "solc = \"0.8.36\"\n").unwrap();

        let config = Config::new()
            .with_root(dir.path())
            .load("ripfuzz.toml")
            .unwrap();

        assert_eq!(config.solc, "0.8.36");
        assert_eq!(config.out, PathBuf::from(".ripfuzz/out"));
    }

    #[test]
    fn load_missing_file_fails_with_resolved_path() {
        let dir = tempfile::tempdir().unwrap();

        let err = Config::new()
            .with_root(dir.path())
            .load("missing.toml")
            .unwrap_err();

        let expected = dir.path().join("missing.toml");
        assert_eq!(
            err.to_string(),
            format!("failed to read config `{}`", expected.display())
        );
    }
}
