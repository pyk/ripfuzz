use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::target_contract::artifact::{TargetContractArtifact, discover_properties};
use crate::target_contract::foundry_artifact::ArtifactJson;
use crate::target_contract::foundry_forge as forge;
use crate::target_contract::foundry_toml::FoundryToml;

/// Builder that resolves a Foundry project into a [`TargetContractArtifact`].
pub struct TargetContractBuilder;

impl TargetContractBuilder {
    /// Build and load the contract artifact.
    ///
    /// `project_path` is the directory containing `foundry.toml`.
    /// `contract_path` is the Solidity source file relative to the project root.
    pub fn build(project_path: &Path, contract_path: &Path) -> Result<TargetContractArtifact> {
        let resolved = project_path.join(contract_path);

        if !resolved.exists() {
            anyhow::bail!(
                "File not found: {} (resolved from {})",
                resolved.display(),
                contract_path.display()
            );
        }

        if resolved.extension() != Some("sol".as_ref()) {
            anyhow::bail!(
                "Expected a Solidity file (.sol), got: {}",
                contract_path.display()
            );
        }

        let contract_path = resolved.canonicalize()?;
        let project_path = project_path.canonicalize()?;

        forge::build(&project_path, &contract_path)?;

        let contract_name = contract_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid contract path"))?;

        let toml_path = project_path.join("foundry.toml");
        let toml_str = fs::read_to_string(&toml_path)?;
        let toml: FoundryToml = toml::from_str(&toml_str)?;
        let profile = toml.default_profile();

        let out_dir = project_path.join(profile.out());

        let artifact_name = Self::resolve_artifact_name(&out_dir, contract_name)?;
        let artifact_path = out_dir
            .join(format!("{contract_name}.sol"))
            .join(&artifact_name);

        let artifact_json: ArtifactJson =
            serde_json::from_str(&fs::read_to_string(&artifact_path)?)?;

        let mut artifact = artifact_json.into_artifact(contract_name.to_string());
        artifact.properties = discover_properties(&artifact.abi);

        Ok(artifact)
    }

    fn resolve_artifact_name(out_dir: &Path, contract_name: &str) -> Result<String> {
        let artifacts = forge::list_artifacts(out_dir, contract_name)?;

        if artifacts.len() == 1 {
            return Ok(artifacts.into_iter().next().unwrap());
        }

        if artifacts.is_empty() {
            anyhow::bail!("no compiled artifacts for contract {}", contract_name);
        }

        // Multiple artifacts -- try to use build-info timestamp to disambiguate.
        match forge::latest_build_info(out_dir)? {
            Some(ts) => {
                let preferred = artifacts.iter().find(|a| a.contains(ts.as_str()));
                match preferred {
                    Some(a) => Ok(a.clone()),
                    None => anyhow::bail!(
                        "multiple artifacts for {} and could not disambiguate: {:?}",
                        contract_name,
                        artifacts
                    ),
                }
            }
            None => anyhow::bail!(
                "multiple artifacts for {} and could not disambiguate: {:?}",
                contract_name,
                artifacts
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_succeeds_with_basic_target() {
        let artifact = TargetContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/Target.sol"),
        )
        .unwrap();
        assert_eq!(artifact.contract_name, "Target");
        assert_eq!(artifact.abi.functions().count(), 3);
    }

    #[test]
    fn build_fails_with_compiler_error() {
        let err = TargetContractBuilder::build(
            Path::new("fixtures/build-failed"),
            Path::new("test/Broken.sol"),
        )
        .unwrap_err();
        let expected = "Error: Compiler run failed:\nError (2314): Expected ';' but got 'function'\n --> test/Broken.sol:7:5:\n  |\n7 |     function set(uint256 x) external {\n  |     ^^^^^^^^";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn build_fails_when_file_not_found() {
        let err = TargetContractBuilder::build(
            Path::new("fixtures/build-failed"),
            Path::new("test/Missing.sol"),
        )
        .unwrap_err();
        let expected = "File not found: fixtures/build-failed/test/Missing.sol (resolved from test/Missing.sol)";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn build_fails_when_not_solidity() {
        let err = TargetContractBuilder::build(
            Path::new("fixtures/build-failed"),
            Path::new("test/something.txt"),
        )
        .unwrap_err();
        let expected = "Expected a Solidity file (.sol), got: test/something.txt";
        assert_eq!(format!("{err}"), expected);
    }
}
