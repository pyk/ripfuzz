//! Remappings resolver for solc compilation.
//!
//! Reads `{root}/remappings.txt` and resolves import paths that match a
//! remapping prefix. With the remapping `ripfuzz/=lib/ripfuzz/src/`, the
//! import `ripfuzz/Harness.sol` resolves to `lib/ripfuzz/src/Harness.sol`
//! relative to the project root.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

/// Resolves Solidity import paths using `{root}/remappings.txt`.
#[derive(Clone, Debug)]
pub struct RemappingsResolver {
    root: PathBuf,
    remappings: Vec<Remapping>,
}

impl Default for RemappingsResolver {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            remappings: Vec::new(),
        }
    }
}

impl RemappingsResolver {
    /// Loads remappings from `{root}/remappings.txt`.
    ///
    /// Returns an empty resolver when the file does not exist.
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let path = root.join("remappings.txt");
        if !path.is_file() {
            return Ok(Self {
                root: root.to_path_buf(),
                remappings: Vec::new(),
            });
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read remappings `{}`", path.display()))?;
        let remappings = parse_remappings(&content)?;
        Ok(Self {
            root: root.to_path_buf(),
            remappings,
        })
    }

    /// Resolves `import` through the first matching remapping.
    ///
    /// Returns the candidate path joined with the project root when the
    /// mapping target is relative, or `None` when no remapping matches.
    pub fn resolve(&self, import: &str) -> Option<PathBuf> {
        let remapping = self
            .remappings
            .iter()
            .find(|remapping| import.starts_with(remapping.prefix.as_str()))?;
        let rest = &import[remapping.prefix.len()..];
        let resolved = PathBuf::from(format!("{}{}", remapping.target, rest));
        if resolved.is_absolute() {
            Some(resolved)
        } else {
            Some(self.root.join(resolved))
        }
    }

    /// Returns the remappings as solc `prefix=target` strings.
    ///
    /// Targets stay relative to the project root, matching the source keys
    /// passed to solc's standard JSON interface.
    pub fn solc_remappings(&self) -> Vec<String> {
        self.remappings
            .iter()
            .map(|remapping| format!("{}={}", remapping.prefix, remapping.target))
            .collect()
    }
}

/// A single remapping from an import prefix to a target directory.
///
/// Both sides keep a trailing slash so prefixes only match whole path
/// segments.
#[derive(Clone, Debug)]
struct Remapping {
    prefix: String,
    target: String,
}

fn with_trailing_slash(value: &str) -> String {
    if value.ends_with('/') {
        value.to_owned()
    } else {
        format!("{value}/")
    }
}

fn parse_remappings(content: &str) -> Result<Vec<Remapping>> {
    let mut remappings = Vec::new();

    for (idx, line) in content.lines().enumerate() {
        let line = line.trim();

        // 1. Skip blank lines and comments.
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }

        // 2. Split the line into prefix and target.
        let Some((prefix, target)) = line.split_once('=') else {
            bail!(
                "invalid remapping on line {}: expected `prefix=target`, got `{line}`",
                idx + 1
            );
        };

        // 3. Reject empty prefixes and targets.
        ensure!(
            !prefix.trim().is_empty() && !target.trim().is_empty(),
            "invalid remapping on line {}: prefix and target must be non-empty",
            idx + 1
        );

        // 4. Normalize trailing slashes so prefixes only match whole path
        //    segments.
        remappings.push(Remapping {
            prefix: with_trailing_slash(prefix.trim()),
            target: with_trailing_slash(target.trim()),
        });
    }

    Ok(remappings)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn load_resolves_imports_via_remappings() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("remappings.txt"),
            "ripfuzz/=lib/ripfuzz/src/\n",
        )
        .unwrap();

        let resolver = RemappingsResolver::load(dir.path()).unwrap();

        assert_eq!(
            resolver.resolve("ripfuzz/Harness.sol"),
            Some(dir.path().join("lib/ripfuzz/src/Harness.sol"))
        );
    }

    #[test]
    fn load_without_file_resolves_nothing() {
        let dir = tempfile::tempdir().unwrap();

        let resolver = RemappingsResolver::load(dir.path()).unwrap();

        assert_eq!(resolver.resolve("ripfuzz/Harness.sol"), None);
    }

    #[test]
    fn resolve_normalizes_missing_trailing_slash() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("remappings.txt"),
            "ripfuzz=lib/ripfuzz/src\n",
        )
        .unwrap();

        let resolver = RemappingsResolver::load(dir.path()).unwrap();

        assert_eq!(
            resolver.resolve("ripfuzz/Harness.sol"),
            Some(dir.path().join("lib/ripfuzz/src/Harness.sol"))
        );
    }

    #[test]
    fn resolve_ignores_unknown_prefix() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("remappings.txt"),
            "ripfuzz/=lib/ripfuzz/src/\n",
        )
        .unwrap();

        let resolver = RemappingsResolver::load(dir.path()).unwrap();

        assert_eq!(resolver.resolve("other/Harness.sol"), None);
    }

    #[test]
    fn solc_remappings_use_root_relative_targets() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("remappings.txt"),
            "ripfuzz/=lib/ripfuzz/src/\n",
        )
        .unwrap();

        let resolver = RemappingsResolver::load(dir.path()).unwrap();

        assert_eq!(
            resolver.solc_remappings(),
            vec!["ripfuzz/=lib/ripfuzz/src/".to_owned()]
        );
    }

    #[test]
    fn parse_line_without_equals_fails() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("remappings.txt"), "ripfuzz\n").unwrap();

        let err = RemappingsResolver::load(dir.path()).unwrap_err();

        assert_eq!(
            err.to_string(),
            "invalid remapping on line 1: expected `prefix=target`, got `ripfuzz`"
        );
    }
}
