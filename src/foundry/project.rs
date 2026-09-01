//! Foundry project wrapper for compilation via `forge build`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail, ensure};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use tracing::{Span, debug, instrument};
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
    #[instrument(skip(self))]
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

        // tracing spans are thread-local, so rayon workers would log outside
        // the caller's span (e.g. build:) without propagating the handle.
        let span = Span::current();
        let parsed: Vec<(PathBuf, Result<(ArtifactId, Artifact)>)> = paths
            .into_par_iter()
            // checkrs: allow(clone_in_iterator)
            .map(|path| {
                let _guard = span.enter();
                debug!(path = %path.display(), "parsing artifact");
                let result = match Artifact::from_json(&path) {
                    Ok(mut artifact) => {
                        artifact.set_project_path(&project_path);
                        let id = artifact.id().clone();
                        Ok((id, artifact))
                    }
                    Err(e) => Err(e),
                };
                (path, result)
            })
            .collect();

        let mut artifacts = HashMap::new();
        for (path, result) in parsed {
            let Ok((id, artifact)) = result else {
                if let Err(e) = result {
                    debug!(path = %path.display(), error = %e, "skipping artifact");
                }
                continue;
            };
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

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn load_artifacts_fails_without_out_dir() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("foundry.toml"), "[profile.default]\n").unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();

        let project = Project::new(temp.path());
        let result = project.load_artifacts();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            format!(
                "output directory does not exist: {}",
                temp.path().join("out").display()
            )
        );
    }
}
