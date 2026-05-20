//! Test helpers for loading fixture artifacts.

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::contract::ContractArtifact;
use crate::foundry::BuildArtifactId;

/// Load a test artifact by its full [`BuildArtifactId`] string.
///
/// Example: `load_test_artifact_by_id("fixtures/basic-target", "test/Target.sol:Target")`.
///
/// **Note:** This assumes fixtures are pre-built (run `make build-fixtures`).
pub fn load_test_artifact_by_id(
    project_path: impl AsRef<Path>,
    id: impl AsRef<str>,
) -> Result<ContractArtifact> {
    let project = crate::foundry::Project {
        path: project_path.as_ref().to_path_buf(),
        force: false,
    };
    let artifacts = project.load_build_artifacts()?;
    let id = BuildArtifactId::try_from(id.as_ref())?;
    let target = artifacts
        .get(&id)
        .context("target not found in build artifacts")?;
    match target {
        crate::foundry::BuildArtifact::Contract(c) => {
            ContractArtifact::from_foundry_artifact(c, &artifacts, &project_path)
        }
        _ => bail!("target must be a concrete contract"),
    }
}

/// Load a test artifact by source file path.
///
/// The path must match exactly one deployable artifact; otherwise an
/// error is returned.
///
/// **Note:** This assumes fixtures are pre-built (run `make build-fixtures`).
pub fn load_test_artifact(
    project_path: impl AsRef<Path>,
    target_path: impl AsRef<Path>,
) -> Result<ContractArtifact> {
    let project = crate::foundry::Project {
        path: project_path.as_ref().to_path_buf(),
        force: false,
    };
    let artifacts = project.load_build_artifacts()?;
    let target_path = target_path.as_ref();

    let candidates: Vec<&crate::foundry::BuildArtifact> = artifacts
        .values()
        .filter(|a| a.id().path == target_path)
        .collect();

    ensure!(
        !candidates.is_empty(),
        "no artifact found for path {}",
        target_path.display()
    );
    ensure!(
        candidates.len() == 1,
        "multiple artifacts found for path {}: {:?}. Move each contract into its own file",
        target_path.display(),
        candidates
            .iter()
            .map(|a| a.name().to_string())
            .collect::<Vec<String>>()
    );

    let target = candidates.into_iter().next().context("no candidate")?;
    match target {
        crate::foundry::BuildArtifact::Contract(c) => {
            ContractArtifact::from_foundry_artifact(c, &artifacts, &project_path)
        }
        _ => bail!("target must be a concrete contract"),
    }
}
