//! `init` CLI command implementation.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::Parser;

const CONFIG_FILE: &str = "ripfuzz.toml";
const GITIGNORE_FILE: &str = ".gitignore";
const CONFIG_CONTENT: &str =
    "[solc]\nversion = \"0.8.36\"\noptimizer = true\noptimizer_runs = 200\n";
const GITIGNORE_CONTENT: &str = ".ripfuzz\n.env\n";

/// Initialize a new ripfuzz project.
#[derive(Debug, Parser)]
pub struct Args {}

impl Args {
    pub fn run(&self) -> Result<()> {
        Initializer::new(".").run()
    }
}

pub fn run(args: Args) -> Result<()> {
    args.run()
}

#[derive(Debug, Clone)]
struct Initializer {
    config_path: PathBuf,
    gitignore_path: PathBuf,
}

impl Initializer {
    fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            config_path: dir.as_ref().join(CONFIG_FILE),
            gitignore_path: dir.as_ref().join(GITIGNORE_FILE),
        }
    }

    fn run(&self) -> Result<()> {
        // 1. Create the config file, refusing to overwrite an existing one.
        ensure!(!self.config_path.exists(), "{CONFIG_FILE} already exists");
        fs::write(&self.config_path, CONFIG_CONTENT)
            .with_context(|| format!("failed to write {}", self.config_path.display()))?;

        // 2. Ensure `.gitignore` ignores ripfuzz artifacts and secrets.
        self.update_gitignore()
    }

    fn update_gitignore(&self) -> Result<()> {
        if !self.gitignore_path.exists() {
            return fs::write(&self.gitignore_path, GITIGNORE_CONTENT)
                .with_context(|| format!("failed to write {}", self.gitignore_path.display()));
        }

        // 2a. Collect the entries missing from the existing `.gitignore`.
        let content = fs::read_to_string(&self.gitignore_path)
            .with_context(|| format!("failed to read {}", self.gitignore_path.display()))?;
        let ignored: Vec<&str> = content.lines().map(str::trim).collect();
        let missing: Vec<&str> = GITIGNORE_CONTENT
            .lines()
            .filter(|entry| !ignored.contains(entry))
            .collect();

        // 2b. Append the missing entries, preserving a trailing newline.
        if missing.is_empty() {
            return Ok(());
        }
        let mut updated = content;
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        for entry in missing {
            updated.push_str(entry);
            updated.push('\n');
        }
        fs::write(&self.gitignore_path, updated)
            .with_context(|| format!("failed to write {}", self.gitignore_path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn initializer_creates_config_with_expected_content() {
        let dir = tempfile::tempdir().unwrap();
        Initializer::new(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert_eq!(content, CONFIG_CONTENT);
    }

    #[test]
    fn initializer_fails_if_config_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CONFIG_FILE), CONFIG_CONTENT).unwrap();
        let initializer = Initializer::new(dir.path());
        let err = initializer.run().unwrap_err();
        assert_eq!(err.to_string(), "ripfuzz.toml already exists");
    }

    #[test]
    fn initializer_creates_gitignore_with_expected_content() {
        let dir = tempfile::tempdir().unwrap();
        Initializer::new(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, ".ripfuzz\n.env\n");
    }

    #[test]
    fn initializer_appends_missing_entries_to_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(GITIGNORE_FILE), "node_modules\n").unwrap();
        Initializer::new(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, "node_modules\n.ripfuzz\n.env\n");
    }

    #[test]
    fn initializer_appends_only_missing_gitignore_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(GITIGNORE_FILE), ".env\n").unwrap();
        Initializer::new(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, ".env\n.ripfuzz\n");
    }

    #[test]
    fn initializer_keeps_gitignore_with_all_entries_untouched() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(GITIGNORE_FILE), ".ripfuzz\n.env\n").unwrap();
        Initializer::new(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, ".ripfuzz\n.env\n");
    }

    #[test]
    fn initializer_appends_after_gitignore_without_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(GITIGNORE_FILE), "node_modules").unwrap();
        Initializer::new(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, "node_modules\n.ripfuzz\n.env\n");
    }
}
