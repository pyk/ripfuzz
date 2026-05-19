//! Builder for resolving Foundry projects into deployable contract artifacts.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use alloy_json_abi::JsonAbi;
use anyhow::{Context, Result, bail, ensure};
use revm::primitives::Bytes;

use crate::contract::artifact;
use crate::foundry::artifact as foundry_artifact;
use crate::foundry::forge;
use crate::foundry::toml as foundry_toml;

/// Scan a Solidity source file for `contract` declarations and return the
/// declared names. Interfaces and libraries are intentionally excluded: they
/// are not deployable targets and should not trigger "multiple contracts"
/// errors when defined alongside a target contract.
fn source_contract_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_block_comment = false;

    for line in source.lines() {
        let line = line.trim();

        if in_block_comment {
            if line.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }

        if line.starts_with("//") {
            continue;
        }

        if line.contains("/*") && !line.contains("*/") {
            in_block_comment = true;
            continue;
        }

        // Strip inline comments.
        let line = line.split("//").next().unwrap_or(line);
        let line = line.split("/*").next().unwrap_or(line);

        let Some(pos) = line.find("contract ") else {
            continue;
        };
        let after = &line[pos + "contract ".len()..];
        let name = after
            .split(|c: char| c.is_whitespace() || c == '{' || c == '(')
            .next()
            .unwrap_or("")
            .trim();
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        {
            names.push(name.into());
        }
    }

    names
}

/// Builder that resolves a Foundry project into a [`artifact::ContractArtifact`].
pub struct ContractBuilder {
    project_path: Option<PathBuf>,
    target_path: Option<PathBuf>,
}

impl ContractBuilder {
    /// Start a builder anchored to a Foundry project directory.
    pub fn for_project(path: impl AsRef<Path>) -> Self {
        Self {
            project_path: Some(path.as_ref().to_path_buf()),
            target_path: None,
        }
    }

    /// Set the Solidity source file path relative to the project root.
    pub fn with_target_path(mut self, path: impl AsRef<Path>) -> Self {
        self.target_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Build and load the contract artifact.
    pub fn build(self) -> Result<artifact::ContractArtifact> {
        let project_path = self.project_path.context("project path is required")?;
        let contract_path = self.target_path.context("target path is required")?;

        let resolved = project_path.join(&contract_path);

        ensure!(
            resolved.exists(),
            "File not found: {} (resolved from {})",
            resolved.display(),
            contract_path.display()
        );

        if resolved.extension() != Some("sol".as_ref()) {
            bail!(
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
            .context("invalid contract path")?;

        let toml_path = project_path.join("foundry.toml");
        let toml_str = fs::read_to_string(&toml_path)?;
        let toml: foundry_toml::FoundryToml = toml::from_str(&toml_str)?;
        let profile = toml.default_profile()?;

        let out_dir = project_path.join(profile.out());

        // Compute the source path relative to the project root for artifact disambiguation.
        let source_path = contract_path
            .strip_prefix(&project_path)
            .unwrap_or(&contract_path)
            .to_string_lossy()
            .replace('\\', "/");

        let source_text = fs::read_to_string(&contract_path)?;
        let source_contracts = source_contract_names(&source_text);

        let artifact_name = Self::resolve_artifact_name(
            &out_dir,
            contract_name,
            source_path.as_str(),
            &source_contracts,
        )?;
        let artifact_path = out_dir
            .join(format!("{contract_name}.sol"))
            .join(&artifact_name);

        let artifact_json: foundry_artifact::ArtifactJson =
            serde_json::from_str(&fs::read_to_string(&artifact_path)?)?;

        // Use the real contract name from the artifact filename (e.g. SimpleKnob.json -> SimpleKnob)
        let contract_name = artifact_name
            .strip_suffix(".json")
            .unwrap_or(&artifact_name);

        let all_contracts = Self::load_all_contracts(&out_dir)?;
        let mut artifact =
            artifact_json.into_artifact_with_all(contract_name, &contract_path, all_contracts);
        artifact.invariants = artifact::find_and_validate_invariants(&artifact.abi)?;

        Ok(artifact)
    }

    fn resolve_artifact_name(
        out_dir: impl AsRef<Path>,
        contract_name: &str,
        source_path: impl AsRef<Path>,
        source_contracts: &[String],
    ) -> Result<String> {
        let artifacts = forge::list_artifacts(&out_dir, contract_name)?;

        if artifacts.len() == 1 {
            let artifact = artifacts
                .into_iter()
                .next()
                .context("expected exactly one artifact")?;
            return Ok(artifact);
        }

        ensure!(
            !artifacts.is_empty(),
            "no compiled artifacts for contract {}",
            contract_name
        );

        // Multiple artifacts -- read each one and match by compilation target.
        // Only keep artifacts whose compilation-target contract name is still
        // declared in the source file. This correctly handles:
        //   * stale artifacts left after a contract rename (old name gone)
        //   * multiple contracts in the same file (error if >1 match)
        let mut candidates = Vec::new();
        for name in &artifacts {
            let path = out_dir
                .as_ref()
                .join(format!("{contract_name}.sol"))
                .join(name);
            let Ok(json_str) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<foundry_artifact::ArtifactJson>(&json_str) else {
                continue;
            };
            let ct_name = if let Some(ref metadata) = json.metadata
                && let Some(ref settings) = metadata.settings
                && let Some(ref targets) = settings.compilation_target
            {
                targets
                    .get(source_path.as_ref().to_string_lossy().as_ref())
                    .cloned()
            } else {
                None
            };
            let Some(ct_name) = ct_name else { continue };
            if source_contracts.contains(&ct_name) {
                candidates.push((name.to_owned(), ct_name));
            }
        }

        match candidates.len() {
            0 => bail!(
                "multiple artifacts for {} and could not disambiguate: {:?}",
                contract_name,
                artifacts
            ),
            1 => {
                let (name, _) = candidates
                    .into_iter()
                    .next()
                    .context("expected one candidate")?;
                Ok(name)
            }
            _ => bail!(
                "multiple contracts found in {}: {:?}. \
                 Move each contract into its own file",
                source_path.as_ref().display(),
                candidates
                    .iter()
                    .map(|(_, n)| n.as_str())
                    .collect::<Vec<&str>>()
            ),
        }
    }

    /// Load every compiled contract artifact found in the Foundry `out` directory.
    fn load_all_contracts(out_dir: impl AsRef<Path>) -> Result<HashMap<String, (Bytes, JsonAbi)>> {
        let mut map = HashMap::new();

        for entry in fs::read_dir(out_dir.as_ref())? {
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

            // Process every `.json` artifact inside the directory.
            for file in fs::read_dir(entry.path())? {
                let file = file?;
                let name = file.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".json") {
                    continue;
                }
                let contract_name = name.strip_suffix(".json").unwrap_or(&name);

                let Ok(json_str) = fs::read_to_string(file.path()) else {
                    continue;
                };
                let Ok(json) = serde_json::from_str::<foundry_artifact::ArtifactJson>(&json_str)
                else {
                    continue;
                };
                let initcode =
                    crate::foundry::artifact::parse_hex(&json.bytecode.object).unwrap_or_default();
                map.insert(contract_name.into(), (initcode, json.abi));
            }
        }

        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_succeeds_with_basic_target() {
        let artifact = ContractBuilder::for_project(Path::new("fixtures/basic-target"))
            .with_target_path(Path::new("test/Target.sol"))
            .build()
            .unwrap();
        assert_eq!(artifact.contract_name, "Target");
        assert_eq!(artifact.abi.functions().count(), 3);
    }

    #[test]
    fn build_uses_contract_name_not_filename() {
        // Regression: NamedMismatch.sol contains `contract DifferentName`,
        // so the artifact name must be "DifferentName", not "NamedMismatch".
        let artifact = ContractBuilder::for_project(Path::new("fixtures/basic-target"))
            .with_target_path(Path::new("src/NamedMismatch.sol"))
            .build()
            .unwrap();
        assert_eq!(artifact.contract_name, "DifferentName");
        assert!(
            artifact.abi.functions().any(|f| f.name == "set"),
            "ABI should contain the set function"
        );
    }

    #[test]
    fn build_ignores_stale_artifact_after_rename() {
        // Regression: after renaming a contract and recompiling, Foundry
        // leaves the old artifact behind. We must pick the one whose
        // compilation target name still exists in the source.
        let project = Path::new("fixtures/basic-target");
        let source = project.join("src/Renamed.sol");
        let saved = fs::read_to_string(&source).unwrap();

        // Step 1: build with name Original (stale artifact exists).
        let original_source = saved.replace("Renamed", "Original");
        fs::write(&source, &original_source).unwrap();
        let artifact1 = ContractBuilder::for_project(project)
            .with_target_path(Path::new("src/Renamed.sol"))
            .build()
            .unwrap();
        assert_eq!(artifact1.contract_name, "Original");

        // Step 2: rename contract in source and rebuild.
        fs::write(&source, &saved).unwrap();
        let artifact2 = ContractBuilder::for_project(project)
            .with_target_path(Path::new("src/Renamed.sol"))
            .build()
            .unwrap();
        assert_eq!(artifact2.contract_name, "Renamed");

        // Restore source.
        fs::write(&source, &saved).unwrap();
    }

    #[test]
    fn build_fails_when_multiple_contracts_in_file() {
        let err = ContractBuilder::for_project(Path::new("fixtures/basic-target"))
            .with_target_path(Path::new("test/MultiContract.sol"))
            .build()
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("multiple contracts found"),
            "expected 'multiple contracts found' error, got: {msg}"
        );
    }

    #[test]
    fn build_fails_on_invariant_not_view_pure() {
        let err = ContractBuilder::for_project(Path::new("fixtures/basic-target"))
            .with_target_path(Path::new("test/PropertiesDiscovery.sol"))
            .build()
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("invariant_not_view"),
            "expected error mentioning invariant_not_view, got: {msg}"
        );
        assert!(
            msg.contains("pure or view"),
            "expected error mentioning 'pure or view', got: {msg}"
        );
    }

    #[test]
    fn build_discovers_invariants_with_correct_signature() {
        let artifact = ContractBuilder::for_project(Path::new("fixtures/basic-target"))
            .with_target_path(Path::new("src/NamedMismatch.sol"))
            .build()
            .unwrap();
        assert_eq!(artifact.contract_name, "DifferentName");

        // NamedMismatch.sol has one valid invariant: invariant_is_set.
        assert_eq!(
            artifact
                .invariants
                .iter()
                .map(|(_, n)| n.as_str())
                .collect::<Vec<&str>>(),
            vec!["invariant_is_set"]
        );
    }

    #[test]
    fn build_fails_with_compiler_error() {
        let err = ContractBuilder::for_project(Path::new("fixtures/build-failed"))
            .with_target_path(Path::new("test/Broken.sol"))
            .build()
            .unwrap_err();
        let expected = "Error: Compiler run failed:\nError (2314): Expected ';' but got 'function'\n --> test/Broken.sol:7:5:\n  |\n7 |     function set(uint256 x) external {\n  |     ^^^^^^^^";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn build_fails_when_file_not_found() {
        let err = ContractBuilder::for_project(Path::new("fixtures/build-failed"))
            .with_target_path(Path::new("test/Missing.sol"))
            .build()
            .unwrap_err();
        let expected = "File not found: fixtures/build-failed/test/Missing.sol (resolved from test/Missing.sol)";
        assert_eq!(format!("{err}"), expected);
    }

    #[test]
    fn build_fails_when_not_solidity() {
        let err = ContractBuilder::for_project(Path::new("fixtures/build-failed"))
            .with_target_path(Path::new("test/something.txt"))
            .build()
            .unwrap_err();
        let expected = "Expected a Solidity file (.sol), got: test/something.txt";
        assert_eq!(format!("{err}"), expected);
    }
}
