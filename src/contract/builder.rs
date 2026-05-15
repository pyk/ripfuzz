use std::collections::HashMap;
use std::fs;
use std::path::Path;

use alloy_json_abi::JsonAbi;
use anyhow::Result;
use revm::primitives::Bytes;

use crate::contract::artifact::{ContractArtifact, discover_properties};
use crate::foundry::artifact::ArtifactJson;
use crate::foundry::forge;
use crate::foundry::toml::FoundryToml;

/// Builder that resolves a Foundry project into a [`ContractArtifact`].
pub struct ContractBuilder;

impl ContractBuilder {
    /// Build and load the contract artifact.
    ///
    /// `project_path` is the directory containing `foundry.toml`.
    /// `contract_path` is the Solidity source file relative to the project root.
    pub fn build(project_path: &Path, contract_path: &Path) -> Result<ContractArtifact> {
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

        let all_contracts = Self::load_all_contracts(&out_dir)?;
        let mut artifact = artifact_json.into_artifact_with_all(contract_name.to_string(), all_contracts);
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

    /// Load every compiled contract artifact found in the Foundry `out` directory.
    fn load_all_contracts(out_dir: &Path) -> Result<HashMap<String, (Bytes, JsonAbi)>> {
        let mut map = HashMap::new();

        for entry in std::fs::read_dir(out_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let dir_name = entry.file_name();
            let dir_name = dir_name.to_string_lossy();
            // Only directories ending in `.sol`
            if !dir_name.ends_with(".sol") {
                continue;
            }
            let contract_name = dir_name.strip_suffix(".sol").unwrap_or(&dir_name);

            // Find the `.json` artifact inside the directory.
            let mut artifact_file = None;
            for file in std::fs::read_dir(entry.path())? {
                let file = file?;
                let name = file.file_name().to_string_lossy().into_owned();
                if name.ends_with(".json") {
                    artifact_file = Some(file.path());
                    break;
                }
            }
            let artifact_path = match artifact_file {
                Some(p) => p,
                None => continue,
            };

            let json_str = std::fs::read_to_string(&artifact_path)?;
            let json: ArtifactJson = serde_json::from_str(&json_str)?;
            let initcode = crate::foundry::artifact::parse_hex(&json.bytecode.object)
                .unwrap_or_default();
            map.insert(contract_name.to_string(), (initcode, json.abi));
        }

        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_succeeds_with_basic_target() {
        let artifact = ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/Target.sol"),
        )
        .unwrap();
        assert_eq!(artifact.contract_name, "Target");
        assert_eq!(artifact.abi.functions().count(), 3);
    }

    #[test]
    fn build_fails_with_compiler_error() {
        let err = ContractBuilder::build(
            Path::new("fixtures/build-failed"),
            Path::new("test/Broken.sol"),
        )
        .unwrap_err();
        let expected = "Error: Compiler run failed:\nError (2314): Expected ';' but got 'function'\n --> test/Broken.sol:7:5:\n  |\n7 |     function set(uint256 x) external {\n  |     ^^^^^^^^";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn build_fails_when_file_not_found() {
        let err = ContractBuilder::build(
            Path::new("fixtures/build-failed"),
            Path::new("test/Missing.sol"),
        )
        .unwrap_err();
        let expected = "File not found: fixtures/build-failed/test/Missing.sol (resolved from test/Missing.sol)";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn build_fails_when_not_solidity() {
        let err = ContractBuilder::build(
            Path::new("fixtures/build-failed"),
            Path::new("test/something.txt"),
        )
        .unwrap_err();
        let expected = "Expected a Solidity file (.sol), got: test/something.txt";
        assert_eq!(format!("{err}"), expected);
    }
}
