//! `init` CLI command implementation.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::Parser;

use crate::logger::Logger;

const CONFIG_FILE: &str = "ripfuzz.toml";
const GITIGNORE_FILE: &str = ".gitignore";
const CONFIG_CONTENT: &str =
    "[solc]\nversion = \"0.8.36\"\noptimizer = true\noptimizer_runs = 200\n";
const GITIGNORE_CONTENT: &str = ".ripfuzz\n.env\n";

/// Initialize a new ripfuzz project.
#[derive(Debug, Parser)]
pub struct Command {}

impl Command {
    pub fn run(&self) -> Result<()> {
        // 1. Initialize stderr logging without a log file so command errors
        //    reach the console without creating `.ripfuzz` state.
        Logger::new().with_root(".").disable_log_file().init()?;

        // 2. Create the project files.
        Initializer::new().with_root(".").run()
    }
}

#[derive(Debug, Clone)]
struct Initializer {
    root: PathBuf,
}

impl Initializer {
    fn new() -> Self {
        Self {
            root: PathBuf::from("."),
        }
    }

    fn with_root(mut self, root: impl AsRef<Path>) -> Self {
        self.root = root.as_ref().to_path_buf();
        self
    }

    fn run(&self) -> Result<()> {
        // 1. Create the config file, refusing to overwrite an existing one.
        let config_path = self.root.join(CONFIG_FILE);
        ensure!(!config_path.exists(), "{CONFIG_FILE} already exists");
        fs::write(&config_path, CONFIG_CONTENT)
            .with_context(|| format!("failed to write {}", config_path.display()))?;

        // 2. Ensure `.gitignore` ignores ripfuzz artifacts and secrets.
        self.update_gitignore()
    }

    fn update_gitignore(&self) -> Result<()> {
        let gitignore_path = self.root.join(GITIGNORE_FILE);
        if !gitignore_path.exists() {
            return fs::write(&gitignore_path, GITIGNORE_CONTENT)
                .with_context(|| format!("failed to write {}", gitignore_path.display()));
        }

        // 2a. Collect the entries missing from the existing `.gitignore`.
        let content = fs::read_to_string(&gitignore_path)
            .with_context(|| format!("failed to read {}", gitignore_path.display()))?;
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
        fs::write(&gitignore_path, updated)
            .with_context(|| format!("failed to write {}", gitignore_path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn initializer_creates_config_with_expected_content() {
        let dir = tempfile::tempdir().unwrap();
        Initializer::new().with_root(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert_eq!(content, CONFIG_CONTENT);
    }

    #[test]
    fn initializer_fails_if_config_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CONFIG_FILE), CONFIG_CONTENT).unwrap();
        let initializer = Initializer::new().with_root(dir.path());
        let err = initializer.run().unwrap_err();
        assert_eq!(err.to_string(), "ripfuzz.toml already exists");
    }

    #[test]
    fn initializer_creates_gitignore_with_expected_content() {
        let dir = tempfile::tempdir().unwrap();
        Initializer::new().with_root(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, ".ripfuzz\n.env\n");
    }

    #[test]
    fn initializer_appends_missing_entries_to_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(GITIGNORE_FILE), "node_modules\n").unwrap();
        Initializer::new().with_root(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, "node_modules\n.ripfuzz\n.env\n");
    }

    #[test]
    fn initializer_appends_only_missing_gitignore_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(GITIGNORE_FILE), ".env\n").unwrap();
        Initializer::new().with_root(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, ".env\n.ripfuzz\n");
    }

    #[test]
    fn initializer_keeps_gitignore_with_all_entries_untouched() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(GITIGNORE_FILE), ".ripfuzz\n.env\n").unwrap();
        Initializer::new().with_root(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, ".ripfuzz\n.env\n");
    }

    #[test]
    fn initializer_appends_after_gitignore_without_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(GITIGNORE_FILE), "node_modules").unwrap();
        Initializer::new().with_root(dir.path()).run().unwrap();
        let content = fs::read_to_string(dir.path().join(GITIGNORE_FILE)).unwrap();
        assert_eq!(content, "node_modules\n.ripfuzz\n.env\n");
    }
}
