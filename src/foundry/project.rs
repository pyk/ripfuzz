//! Foundry project wrapper for compilation via `forge build`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail, ensure};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use tracing::{debug, instrument};
use walkdir::WalkDir;

use crate::foundry::artifact::{Artifact, ArtifactId};
use crate::foundry::build_options::BuildOptions;

/// A Foundry project located at a specific filesystem path.
#[derive(Debug, Clone)]
pub struct Project {
    /// Absolute or relative path to the project root.
    pub path: PathBuf,
}

impl Project {
    /// Create a [`Project`] without compiling.
    ///
    /// Construction is cheap and infallible. Call [`Self::build`] to compile
    /// and [`Self::load_artifacts`] to read the compiled artifacts.
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Compile the project via `forge build`.
    ///
    /// The `out/` directory is created or refreshed as a side effect.
    pub fn build(&self, opts: BuildOptions) -> Result<()> {
        let mut cmd = Command::new("forge");
        cmd.arg("build")
            .arg("--ast")
            .arg("--extra-output")
            .arg("storageLayout")
            .arg("--root")
            .arg(&self.path);

        if opts.is_force() {
            cmd.arg("--force");
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("{}", stderr.trim());
        }

        Ok(())
    }

    /// Load all build artifacts from the project's `out/` directory.
    ///
    /// Returns a map keyed by [`ArtifactId`]. The `out/` directory must
    /// already exist (compile first if necessary).
    ///
    /// Artifact JSONs are discovered with `walkdir` (`out/*.sol/*.json`) and
    /// parsed in parallel with `rayon`.
    #[instrument(fields(path = %self.path.display()))]
    pub fn load_artifacts(&self) -> Result<HashMap<ArtifactId, Artifact>> {
        let out_dir = self.path.join("out");
        ensure!(
            out_dir.exists(),
            "output directory does not exist: {}",
            out_dir.display()
        );

        debug!(out_dir = %out_dir.display(), "discovering build artifacts");

        let paths: Vec<PathBuf> = WalkDir::new(&out_dir)
            .min_depth(1)
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if !entry.file_type().is_file() {
                    return None;
                }
                if path.extension()? != "json" {
                    return None;
                }
                let parent = path.parent()?;
                if !parent.file_name()?.to_str()?.ends_with(".sol") {
                    return None;
                }
                Some(path.to_path_buf())
            })
            .collect();

        debug!(count = paths.len(), "found artifact files");

        let project_path = self.path.canonicalize().unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|cwd| cwd.join(&self.path))
                .unwrap_or_else(|_| self.path.clone())
        });

        let parsed: Vec<Result<(ArtifactId, Artifact)>> = paths
            .into_par_iter()
            // checkrs: allow(clone_in_iterator)
            .map(|path| {
                debug!(path = %path.display(), "parsing artifact");
                let mut artifact = Artifact::from_json(&path)?;
                artifact.set_project_path(&project_path);
                let id = artifact.id().clone();
                Ok((id, artifact))
            })
            .collect();

        let mut artifacts = HashMap::new();
        for result in parsed {
            let (id, artifact) = result?;
            if artifacts.contains_key(&id) {
                bail!("duplicate build artifact id: {}", id);
            }
            artifacts.insert(id, artifact);
        }

        Ok(artifacts)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serial_test::serial;
    use tempfile::TempDir;

    use super::*;

    #[test]
    #[serial]
    fn build_succeeds() {
        let project = Project::new("fixtures/foundry-project");
        let result = project.build(BuildOptions::new());
        assert!(result.is_ok());
    }

    #[test]
    fn build_fails() {
        let project = Project::new("fixtures/build-failed");
        let result = project.build(BuildOptions::new());
        assert!(result.is_err());
    }

    #[test]
    fn load_artifacts_succeeds() {
        let project = Project::new("fixtures/foundry-project");
        let artifacts = project.load_artifacts().unwrap();
        assert_eq!(artifacts.len(), 4);

        let counter_id = ArtifactId::try_from("src/Counter.sol:Counter").unwrap();
        let counter = artifacts
            .get(&counter_id)
            .expect("Counter artifact missing");
        assert!(matches!(counter, Artifact::Contract(_)));
        assert!(!counter.project_path().as_os_str().is_empty());

        let icounter_id = ArtifactId::try_from("src/ICounter.sol:ICounter").unwrap();
        let icounter = artifacts
            .get(&icounter_id)
            .expect("ICounter artifact missing");
        assert!(matches!(icounter, Artifact::Interface(_)));
        assert!(!icounter.project_path().as_os_str().is_empty());

        let lib_id = ArtifactId::try_from("src/CounterLib.sol:CounterLib").unwrap();
        let lib = artifacts.get(&lib_id).expect("CounterLib artifact missing");
        assert!(matches!(lib, Artifact::Library(_)));
        assert!(!lib.project_path().as_os_str().is_empty());

        let abs_id = ArtifactId::try_from("src/AbstractCounter.sol:AbstractCounter").unwrap();
        let abs = artifacts
            .get(&abs_id)
            .expect("AbstractCounter artifact missing");
        assert!(matches!(abs, Artifact::Abstract(_)));
        assert!(!abs.project_path().as_os_str().is_empty());
    }

    #[test]
    fn parse_contract_artifact() {
        // Fixture must be pre-built (run `make build-fixtures`).
        let json =
            fs::read_to_string("fixtures/foundry-project/out/Counter.sol/Counter.json").unwrap();
        let artifact = Artifact::from_json_str(&json).unwrap();
        assert_eq!(artifact.name(), "Counter");
        assert!(matches!(artifact, Artifact::Contract(_)));
        assert_eq!(artifact.id().to_string(), "src/Counter.sol:Counter");
        assert!(!artifact.abi().functions().next().is_none());

        let Artifact::Contract(contract) = &artifact else {
            panic!("expected Contract artifact");
        };
        assert!(!contract.bytecode.object.is_empty());
        assert!(!contract.deployed_bytecode.object.is_empty());
        assert!(!contract.bytecode.source_map.is_empty());
        assert!(!contract.deployed_bytecode.source_map.is_empty());
    }

    #[test]
    fn parse_interface_artifact() {
        // Fixture must be pre-built (run `make build-fixtures`).
        let json =
            fs::read_to_string("fixtures/foundry-project/out/ICounter.sol/ICounter.json").unwrap();
        let artifact = Artifact::from_json_str(&json).unwrap();
        assert_eq!(artifact.name(), "ICounter");
        assert!(matches!(artifact, Artifact::Interface(_)));
        assert_eq!(artifact.id().to_string(), "src/ICounter.sol:ICounter");

        assert!(matches!(artifact, Artifact::Interface(_)));
    }

    #[test]
    fn parse_library_artifact() {
        // Fixture must be pre-built (run `make build-fixtures`).
        let json =
            fs::read_to_string("fixtures/foundry-project/out/CounterLib.sol/CounterLib.json")
                .unwrap();
        let artifact = Artifact::from_json_str(&json).unwrap();
        assert_eq!(artifact.name(), "CounterLib");
        assert!(matches!(artifact, Artifact::Library(_)));
        assert_eq!(artifact.id().to_string(), "src/CounterLib.sol:CounterLib");

        let Artifact::Library(lib) = &artifact else {
            panic!("expected Library artifact");
        };
        assert!(!lib.bytecode.object.is_empty());
        assert!(!lib.deployed_bytecode.object.is_empty());
        assert!(!lib.bytecode.source_map.is_empty());
        assert!(!lib.deployed_bytecode.source_map.is_empty());
    }

    #[test]
    fn parse_abstract_artifact() {
        // Fixture must be pre-built (run `make build-fixtures`).
        let json = fs::read_to_string(
            "fixtures/foundry-project/out/AbstractCounter.sol/AbstractCounter.json",
        )
        .unwrap();
        let artifact = Artifact::from_json_str(&json).unwrap();
        assert_eq!(artifact.name(), "AbstractCounter");
        assert!(matches!(artifact, Artifact::Abstract(_)));
        assert_eq!(
            artifact.id().to_string(),
            "src/AbstractCounter.sol:AbstractCounter"
        );

        assert!(matches!(artifact, Artifact::Abstract(_)));
    }

    #[test]
    fn load_artifacts_fails_without_out_dir() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("foundry.toml"), "[profile.default]\n").unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();

        let project = Project::new(temp.path());
        let result = project.load_artifacts();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("output directory does not exist")
        );
    }

    #[test]
    fn load_artifacts_skips_build_info() {
        let project = Project::new("fixtures/foundry-project");
        let artifacts = project.load_artifacts().unwrap();
        // build-info JSONs should be skipped, so we only have contract artifacts
        assert_eq!(artifacts.len(), 4);
        for (id, artifact) in &artifacts {
            assert!(!id.path.as_os_str().is_empty());
            assert!(!id.name.is_empty());
            assert_eq!(id.name, artifact.name());
            assert!(!artifact.project_path().as_os_str().is_empty());
        }
    }
}
