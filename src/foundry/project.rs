//! Foundry project wrapper for compilation via `forge build`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail};

/// A Foundry project located at a specific filesystem path.
pub struct Project {
    /// Absolute or relative path to the project root.
    pub path: PathBuf,
}

impl Project {
    /// Create a new [`Project`] pointing to `path`.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Run `forge build --root <project_path>`.
    ///
    /// Returns an error containing the `stderr` output when `forge build` fails.
    pub fn build(&self) -> Result<()> {
        let output = Command::new("forge")
            .arg("build")
            .arg("--ast")
            .arg("--root")
            .arg(&self.path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("{}", stderr.trim());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_succeeds() {
        let project = Project::new("fixtures/foundry-project");
        assert!(project.build().is_ok());
    }

    #[test]
    fn build_fails() {
        let project = Project::new("fixtures/build-failed");
        let result = project.build();
        assert!(result.is_err());
    }
}
