//! Harness identifier for `ripfuzz max`.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Result, ensure};

/// Unique identifier for a harness used by `ripfuzz max`.
///
/// Accepted forms:
/// - `src/MyHarness.sol` (contract name derived from file stem)
/// - `src/MyHarness.sol:MyHarness` (explicit contract name)
///
/// The path is parsed only and is not required to exist.
/// Consumers resolve it against the project root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessId {
    pub path: PathBuf,
    pub name: String,
}

impl TryFrom<String> for HarnessId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl TryFrom<&str> for HarnessId {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        ensure!(!trimmed.is_empty(), "harness must be non-empty");

        if trimmed.contains(':') {
            let parts: Vec<&str> = trimmed.splitn(2, ':').collect();
            ensure!(
                parts.len() == 2,
                "expected format `src/MyHarness.sol` or `src/MyHarness.sol:MyHarness`, got `{}`",
                trimmed
            );
            let path_str = parts[0];
            let name = parts[1];
            ensure!(
                !path_str.is_empty() && !name.is_empty(),
                "path and contract name must be non-empty"
            );
            ensure!(!name.contains(':'), "contract name must not contain colon");
            let path = PathBuf::from(path_str);
            ensure!(
                path.extension().is_some_and(|ext| ext == "sol"),
                "path must end with `.sol`, got `{}`",
                path.display()
            );
            Ok(Self {
                path,
                name: name.to_owned(),
            })
        } else {
            let path = PathBuf::from(trimmed);
            ensure!(
                path.extension().is_some_and(|ext| ext == "sol"),
                "harness must end with `.sol` (e.g. `src/MyHarness.sol`), got `{}`",
                trimmed
            );
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned();
            ensure!(!stem.is_empty(), "path and contract name must be non-empty");
            Ok(Self { path, name: stem })
        }
    }
}

impl FromStr for HarnessId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

impl fmt::Display for HarnessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.name)
    }
}

impl From<HarnessId> for String {
    fn from(id: HarnessId) -> Self {
        id.to_string()
    }
}

impl From<&HarnessId> for String {
    fn from(id: &HarnessId) -> Self {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    fn create_sol_file(dir: impl AsRef<Path>, name: &str) -> PathBuf {
        let path = dir.as_ref().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, "// dummy").unwrap();
        path
    }

    #[test]
    fn harness_id_from_path_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_sol_file(dir.path(), "MyHarness.sol");
        let id = HarnessId::try_from(path.to_str().unwrap()).unwrap();
        assert_eq!(id.path, path);
        assert_eq!(id.name, "MyHarness");
    }

    #[test]
    fn harness_id_from_path_and_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_sol_file(dir.path(), "MyHarness.sol");
        let spec = format!("{}:MyHarness", path.display());
        let id = HarnessId::try_from(spec.as_str()).unwrap();
        assert_eq!(id.path, path);
        assert_eq!(id.name, "MyHarness");
    }

    #[test]
    fn harness_id_from_string_owned() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_sol_file(dir.path(), "MyHarness.sol");
        let spec = format!("{}:MyHarness", path.display());
        let id = HarnessId::try_from(spec.clone()).unwrap();
        assert_eq!(id.path, path);
        assert_eq!(id.name, "MyHarness");
    }

    #[test]
    fn harness_id_display() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_sol_file(dir.path(), "MyHarness.sol");
        let spec = format!("{}:MyHarness", path.display());
        let id = HarnessId::try_from(spec.as_str()).unwrap();
        assert_eq!(id.to_string(), format!("{}:MyHarness", path.display()));
    }

    #[test]
    fn harness_id_display_path_only_canonicalizes() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_sol_file(dir.path(), "MyHarness.sol");
        let id = HarnessId::try_from(path.to_str().unwrap()).unwrap();
        assert_eq!(id.to_string(), format!("{}:MyHarness", path.display()));
    }

    #[test]
    fn harness_id_from_str() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_sol_file(dir.path(), "MyHarness.sol");
        let spec = format!("{}:MyHarness", path.display());
        let id: HarnessId = spec.parse().unwrap();
        assert_eq!(id.path, path);
        assert_eq!(id.name, "MyHarness");
    }

    #[test]
    fn harness_id_nested_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_sol_file(dir.path(), "nested/MyHarness.sol");
        let id = HarnessId::try_from(path.to_str().unwrap()).unwrap();
        assert_eq!(id.path, path);
        assert_eq!(id.name, "MyHarness");
    }

    #[test]
    fn harness_id_empty_fails() {
        let err = HarnessId::try_from("").unwrap_err();
        assert_eq!(err.to_string(), "harness must be non-empty");
    }

    #[test]
    fn harness_id_whitespace_fails() {
        let err = HarnessId::try_from("   ").unwrap_err();
        assert_eq!(err.to_string(), "harness must be non-empty");
    }

    #[test]
    fn harness_id_bare_name_fails() {
        let err = HarnessId::try_from("MyHarness").unwrap_err();
        assert_eq!(
            err.to_string(),
            "harness must end with `.sol` (e.g. `src/MyHarness.sol`), got `MyHarness`"
        );
    }

    #[test]
    fn harness_id_missing_sol_extension_fails() {
        let err = HarnessId::try_from("src/MyHarness").unwrap_err();
        assert_eq!(
            err.to_string(),
            "harness must end with `.sol` (e.g. `src/MyHarness.sol`), got `src/MyHarness`"
        );
    }

    #[test]
    fn harness_id_wrong_extension_fails() {
        let err = HarnessId::try_from("src/MyHarness.txt").unwrap_err();
        assert_eq!(
            err.to_string(),
            "harness must end with `.sol` (e.g. `src/MyHarness.sol`), got `src/MyHarness.txt`"
        );
    }

    #[test]
    fn harness_id_empty_path_fails() {
        let err = HarnessId::try_from(":MyHarness").unwrap_err();
        assert_eq!(err.to_string(), "path and contract name must be non-empty");
    }

    #[test]
    fn harness_id_empty_name_fails() {
        let err = HarnessId::try_from("src/MyHarness.sol:").unwrap_err();
        assert_eq!(err.to_string(), "path and contract name must be non-empty");
    }

    #[test]
    fn harness_id_path_without_sol_in_full_form_fails() {
        let err = HarnessId::try_from("src/MyHarness.txt:MyHarness").unwrap_err();
        assert_eq!(
            err.to_string(),
            "path must end with `.sol`, got `src/MyHarness.txt`"
        );
    }

    #[test]
    fn harness_id_multiple_colons_fails() {
        let err = HarnessId::try_from("src/MyHarness.sol:MyHarness:Extra").unwrap_err();
        assert_eq!(err.to_string(), "contract name must not contain colon");
    }

    #[test]
    fn harness_id_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = create_sol_file(dir.path(), "MyHarness.sol");
        let spec = format!("  {}:MyHarness  ", path.display());
        let id = HarnessId::try_from(spec.as_str()).unwrap();
        assert_eq!(id.path, path);
        assert_eq!(id.name, "MyHarness");
    }
}
