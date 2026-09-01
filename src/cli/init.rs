//! `init` CLI command implementation.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::Parser;

/// Initialize a new ripfuzz project.
#[derive(Debug, Parser)]
pub struct Args {}

impl Args {
    pub fn run(&self) -> Result<()> {
        Initializer::new("ripfuzz.toml").run()
    }
}

pub fn run(args: Args) -> Result<()> {
    args.run()
}

#[derive(Debug, Clone)]
struct Initializer {
    path: PathBuf,
}

impl Initializer {
    fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    fn content(&self) -> String {
        String::from("[solc]\nversion = \"0.8.36\"\noptimizer = true\noptimizer_runs = 200\n")
    }

    fn run(&self) -> Result<()> {
        ensure!(!self.path.exists(), "ripfuzz.toml already exists");
        fs::write(&self.path, self.content())
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn initializer_creates_file_with_expected_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ripfuzz.toml");
        let initializer = Initializer::new(&path);
        initializer.run().unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "[solc]\nversion = \"0.8.36\"\noptimizer = true\noptimizer_runs = 200\n"
        );
    }

    #[test]
    fn initializer_fails_if_file_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ripfuzz.toml");
        fs::write(
            &path,
            "[solc]\nversion = \"0.8.36\"\noptimizer = true\noptimizer_runs = 200\n",
        )
        .unwrap();
        let initializer = Initializer::new(&path);
        let err = initializer.run().unwrap_err();
        assert_eq!(err.to_string(), "ripfuzz.toml already exists");
    }

    #[test]
    fn initializer_content_is_expected() {
        let initializer = Initializer::new("ripfuzz.toml");
        assert_eq!(
            initializer.content(),
            "[solc]\nversion = \"0.8.36\"\noptimizer = true\noptimizer_runs = 200\n"
        );
    }
}
