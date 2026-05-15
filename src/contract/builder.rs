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

/// Scan a Solidity source file for `contract`, `interface`, and `library`
/// declarations and return the declared names.
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

        for keyword in ["contract ", "interface ", "library "] {
            if let Some(pos) = line.find(keyword) {
                let after = &line[pos + keyword.len()..];
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
                    names.push(name.to_string());
                }
                break;
            }
        }
    }

    names
}

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

        // Compute the source path relative to the project root for artifact disambiguation.
        let source_path = contract_path
            .strip_prefix(&project_path)
            .unwrap_or(&contract_path)
            .to_string_lossy()
            .replace('\\', "/");

        let source_text = fs::read_to_string(&contract_path)?;
        let source_contracts = source_contract_names(&source_text);

        let artifact_name =
            Self::resolve_artifact_name(&out_dir, contract_name, &source_path, &source_contracts)?;
        let artifact_path = out_dir
            .join(format!("{contract_name}.sol"))
            .join(&artifact_name);

        let artifact_json: ArtifactJson =
            serde_json::from_str(&fs::read_to_string(&artifact_path)?)?;

        // Use the real contract name from the artifact filename (e.g. SimpleKnob.json -> SimpleKnob)
        let contract_name = artifact_name
            .strip_suffix(".json")
            .unwrap_or(&artifact_name);

        let all_contracts = Self::load_all_contracts(&out_dir)?;
        let mut artifact =
            artifact_json.into_artifact_with_all(contract_name.to_string(), all_contracts);
        artifact.properties = discover_properties(&artifact.abi);

        Ok(artifact)
    }

    fn resolve_artifact_name(
        out_dir: &Path,
        contract_name: &str,
        source_path: &str,
        source_contracts: &[String],
    ) -> Result<String> {
        let artifacts = forge::list_artifacts(out_dir, contract_name)?;

        if artifacts.len() == 1 {
            return Ok(artifacts.into_iter().next().unwrap());
        }

        if artifacts.is_empty() {
            anyhow::bail!("no compiled artifacts for contract {}", contract_name);
        }

        // Multiple artifacts -- read each one and match by compilation target.
        // Only keep artifacts whose compilation-target contract name is still
        // declared in the source file. This correctly handles:
        //   * stale artifacts left after a contract rename (old name gone)
        //   * multiple contracts in the same file (error if >1 match)
        let mut candidates = Vec::new();
        for name in &artifacts {
            let path = out_dir.join(format!("{contract_name}.sol")).join(name);
            let json_str = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let json: ArtifactJson = match serde_json::from_str(&json_str) {
                Ok(j) => j,
                Err(_) => continue,
            };
            let ct_name = if let Some(ref metadata) = json.metadata
                && let Some(ref settings) = metadata.settings
                && let Some(ref targets) = settings.compilation_target
            {
                targets.get(source_path).cloned()
            } else {
                None
            };
            let Some(ct_name) = ct_name else { continue };
            if source_contracts.contains(&ct_name) {
                candidates.push((name.clone(), ct_name));
            }
        }

        match candidates.len() {
            0 => anyhow::bail!(
                "multiple artifacts for {} and could not disambiguate: {:?}",
                contract_name,
                artifacts
            ),
            1 => Ok(candidates.into_iter().next().unwrap().0),
            _ => anyhow::bail!(
                "multiple contracts found in {}: {:?}. \
                 Specify which contract to fuzz with --contract",
                source_path,
                candidates.iter().map(|(_, n)| n).collect::<Vec<_>>()
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

            // Process every `.json` artifact inside the directory.
            for file in std::fs::read_dir(entry.path())? {
                let file = file?;
                let name = file.file_name().to_string_lossy().into_owned();
                if !name.ends_with(".json") {
                    continue;
                }
                let contract_name = name.strip_suffix(".json").unwrap_or(&name);

                let json_str = match std::fs::read_to_string(file.path()) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let json: ArtifactJson = match serde_json::from_str(&json_str) {
                    Ok(j) => j,
                    Err(_) => continue,
                };
                let initcode =
                    crate::foundry::artifact::parse_hex(&json.bytecode.object).unwrap_or_default();
                map.insert(contract_name.to_string(), (initcode, json.abi));
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
        let artifact = ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/Target.sol"),
        )
        .unwrap();
        assert_eq!(artifact.contract_name, "Target");
        assert_eq!(artifact.abi.functions().count(), 3);
    }

    #[test]
    fn build_uses_contract_name_not_filename() {
        // Regression: NamedMismatch.sol contains `contract DifferentName`,
        // so the artifact name must be "DifferentName", not "NamedMismatch".
        let artifact = ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("src/NamedMismatch.sol"),
        )
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
        let original = fs::read_to_string(&source).unwrap();

        // Step 1: build with current name Original.
        let artifact1 = ContractBuilder::build(project, Path::new("src/Renamed.sol")).unwrap();
        assert_eq!(artifact1.contract_name, "Original");

        // Step 2: rename contract in source and rebuild.
        let renamed = original.replace("Original", "Renamed");
        fs::write(&source, &renamed).unwrap();
        let artifact2 = ContractBuilder::build(project, Path::new("src/Renamed.sol")).unwrap();
        assert_eq!(artifact2.contract_name, "Renamed");

        // Restore original source.
        fs::write(&source, &original).unwrap();
    }

    #[test]
    fn build_fails_when_multiple_contracts_in_file() {
        let err = ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/MultiContract.sol"),
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("multiple contracts found"),
            "expected 'multiple contracts found' error, got: {msg}"
        );
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
