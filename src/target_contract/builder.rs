use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::target_contract::artifact::ArtifactJson;
use crate::target_contract::config::FoundryToml;
use crate::target_contract::contract::{TargetContract, discover_properties};
use crate::target_contract::forge;

/// Builder that resolves a Foundry project into a [`TargetContract`].
pub struct TargetContractBuilder;

impl TargetContractBuilder {
    /// Build the contract at `contract_path`.
    ///
    /// The path must be a `.sol` file somewhere inside a Foundry project.  The
    /// builder walks upward until it finds `foundry.toml`, uses that directory
    /// as the project root, and runs `forge build <contract_path>`.
    ///
    /// If `project_root` is `Some`, the auto-discovery step is skipped and the
    /// provided directory is used instead.
    pub fn build(contract_path: &Path, project_root: Option<&Path>) -> Result<TargetContract> {
        if !contract_path.exists() {
            anyhow::bail!("File not found: {}", contract_path.display());
        }

        if contract_path.extension() != Some("sol".as_ref()) {
            anyhow::bail!(
                "Expected a Solidity file (.sol), got: {}",
                contract_path.display()
            );
        }

        let contract_path = contract_path.canonicalize()?;
        let project_root = project_root
            .map(Path::to_path_buf)
            .or_else(|| find_project_root(&contract_path))
            .ok_or_else(|| {
                anyhow::anyhow!("could not find foundry.toml in any parent directory")
            })?;

        forge::build(&project_root, &contract_path)?;

        let contract_name = contract_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid contract path"))?;

        let toml_path = project_root.join("foundry.toml");
        let toml_str = fs::read_to_string(&toml_path)?;
        let toml: FoundryToml = toml::from_str(&toml_str)?;
        let profile = toml.default_profile();

        let out_dir = project_root.join(profile.out());

        let artifact_name = Self::resolve_artifact_name(&out_dir, contract_name)?;
        let artifact_path = out_dir
            .join(format!("{contract_name}.sol"))
            .join(&artifact_name);

        let artifact_json: ArtifactJson =
            serde_json::from_str(&fs::read_to_string(&artifact_path)?)?;

        let mut target = artifact_json.into_target(Default::default());
        target.properties = discover_properties(&target.abi);

        Ok(target)
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

fn find_project_root(path: &Path) -> Option<PathBuf> {
    let mut dir = path.parent()?;
    loop {
        if dir.join("foundry.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_succeeds_with_basic_target() {
        let target =
            TargetContractBuilder::build(Path::new("fixtures/basic-target/test/Target.sol"), None)
                .unwrap();
        assert_eq!(target.abi.functions().count(), 3);
    }

    #[test]
    fn build_fails_with_compiler_error() {
        let err =
            TargetContractBuilder::build(Path::new("fixtures/build-failed/test/Broken.sol"), None)
                .unwrap_err();
        let expected = "Error: Compiler run failed:\nError (2314): Expected ';' but got 'function'\n --> test/Broken.sol:7:5:\n  |\n7 |     function set(uint256 x) external {\n  |     ^^^^^^^^";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn build_fails_when_file_not_found() {
        let err =
            TargetContractBuilder::build(Path::new("fixtures/build-failed/test/Missing.sol"), None)
                .unwrap_err();
        let expected = "File not found: fixtures/build-failed/test/Missing.sol";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn build_fails_when_not_solidity() {
        let err = TargetContractBuilder::build(
            Path::new("fixtures/build-failed/test/something.txt"),
            None,
        )
        .unwrap_err();
        let expected =
            "Expected a Solidity file (.sol), got: fixtures/build-failed/test/something.txt";
        assert_eq!(format!("{err}"), expected);
    }
}
