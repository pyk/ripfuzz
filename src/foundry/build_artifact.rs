//! Foundry build artifact types and parsing.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use alloy_json_abi::JsonAbi;
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use tracing::{debug, instrument};

// ---------------------------------------------------------------------------
// Foundry's Build Artifact ID
// ---------------------------------------------------------------------------

/// Unique identifier for a compiled build artifact.
///
/// Format: `path:name` (e.g. `src/Counter.sol:Counter`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BuildArtifactId {
    pub path: PathBuf,
    pub name: String,
}

impl fmt::Display for BuildArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.name)
    }
}

impl TryFrom<String> for BuildArtifactId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let parts: Vec<&str> = value.splitn(2, ':').collect();
        ensure!(
            parts.len() == 2,
            "invalid build artifact id: expected format `path:name`, got `{}`",
            value
        );
        let path = PathBuf::from(parts[0]);
        let name = parts[1].to_owned();
        ensure!(
            !path.as_os_str().is_empty() && !name.is_empty(),
            "invalid build artifact id: path and name must be non-empty"
        );
        Ok(Self { path, name })
    }
}

impl TryFrom<&str> for BuildArtifactId {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl FromStr for BuildArtifactId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

// ---------------------------------------------------------------------------
// Foundry's Build Artifact
// ---------------------------------------------------------------------------

/// A compiled Solidity artifact loaded from a Foundry project.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildArtifact {
    Contract(ContractArtifact),
    Interface(InterfaceArtifact),
    Library(LibraryArtifact),
    Abstract(AbstractArtifact),
}

/// A concrete contract artifact with all data required for fuzzing.
#[derive(Clone, Debug, PartialEq)]
pub struct ContractArtifact {
    pub id: BuildArtifactId,
    pub ast: solc::ast::SourceUnit,
    pub abi: JsonAbi,
    pub bytecode: BuildArtifactBytecode,
    pub deployed_bytecode: BuildArtifactBytecode,
}

/// An interface artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceArtifact {
    pub id: BuildArtifactId,
    pub ast: solc::ast::SourceUnit,
    pub abi: JsonAbi,
}

/// A library artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryArtifact {
    pub id: BuildArtifactId,
    pub ast: solc::ast::SourceUnit,
    pub abi: JsonAbi,
    pub bytecode: BuildArtifactBytecode,
    pub deployed_bytecode: BuildArtifactBytecode,
}

/// An abstract contract artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct AbstractArtifact {
    pub id: BuildArtifactId,
    pub ast: solc::ast::SourceUnit,
    pub abi: JsonAbi,
}

#[derive(Debug, Clone, Deserialize)]
struct BuildArtifactJson {
    abi: JsonAbi,
    bytecode: BuildArtifactBytecode,
    #[serde(rename = "deployedBytecode")]
    deployed_bytecode: BuildArtifactBytecode,
    ast: solc::ast::SourceUnit,
    metadata: Option<BuildArtifactMetadata>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct BuildArtifactBytecode {
    #[serde(default)]
    pub object: String,
    #[serde(default, rename = "sourceMap")]
    pub source_map: String,
    #[serde(default, rename = "linkReferences")]
    pub link_references: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct BuildArtifactMetadata {
    settings: Option<BuildArtifactSettings>,
}

#[derive(Debug, Clone, Deserialize)]
struct BuildArtifactSettings {
    #[serde(rename = "compilationTarget")]
    compilation_target: Option<HashMap<String, String>>,
}

impl BuildArtifact {
    /// Load a build artifact from a JSON file on disk.
    #[instrument(err, fields(path = %path.as_ref().display()))]
    pub fn from_json(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        debug!(path = %path.display(), "loading build artifact");
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read artifact: {}", path.display()))?;
        Self::from_json_str(&content)
            .with_context(|| format!("failed to parse artifact: {}", path.display()))
    }

    /// Load a build artifact from a JSON string.
    #[instrument(err)]
    pub fn from_json_str(content: &str) -> Result<Self> {
        let json: BuildArtifactJson = serde_json::from_str(content)?;

        let id = get_build_artifact_id(&json)?;

        let def = get_contract_definition(&json.ast, &id.name)?;
        Ok(match def.contract_kind {
            solc::ast::ContractKind::Contract if !def.r#abstract => {
                Self::Contract(ContractArtifact {
                    id,
                    ast: json.ast,
                    abi: json.abi,
                    bytecode: json.bytecode,
                    deployed_bytecode: json.deployed_bytecode,
                })
            }
            solc::ast::ContractKind::Contract => Self::Abstract(AbstractArtifact {
                id,
                ast: json.ast,
                abi: json.abi,
            }),
            solc::ast::ContractKind::Interface => Self::Interface(InterfaceArtifact {
                id,
                ast: json.ast,
                abi: json.abi,
            }),
            solc::ast::ContractKind::Library => Self::Library(LibraryArtifact {
                id,
                ast: json.ast,
                abi: json.abi,
                bytecode: json.bytecode,
                deployed_bytecode: json.deployed_bytecode,
            }),
        })
    }

    /// The unique identifier of this artifact.
    pub fn id(&self) -> &BuildArtifactId {
        match self {
            Self::Contract(a) => &a.id,
            Self::Interface(a) => &a.id,
            Self::Library(a) => &a.id,
            Self::Abstract(a) => &a.id,
        }
    }

    /// The unique identifier of this artifact.
    pub fn name(&self) -> &str {
        match self {
            Self::Contract(a) => &a.id.name,
            Self::Interface(a) => &a.id.name,
            Self::Library(a) => &a.id.name,
            Self::Abstract(a) => &a.id.name,
        }
    }

    /// The parsed AST of the source unit.
    pub fn ast(&self) -> &solc::ast::SourceUnit {
        match self {
            Self::Contract(a) => &a.ast,
            Self::Interface(a) => &a.ast,
            Self::Library(a) => &a.ast,
            Self::Abstract(a) => &a.ast,
        }
    }

    /// The JSON ABI.
    pub fn abi(&self) -> &JsonAbi {
        match self {
            Self::Contract(a) => &a.abi,
            Self::Interface(a) => &a.abi,
            Self::Library(a) => &a.abi,
            Self::Abstract(a) => &a.abi,
        }
    }
}

/// Extract `BuildArtifactId` from the artifact metadata.
fn get_build_artifact_id(json: &BuildArtifactJson) -> Result<BuildArtifactId> {
    let target = json
        .metadata
        .as_ref()
        .and_then(|m| m.settings.as_ref())
        .and_then(|s| s.compilation_target.as_ref())
        .context("missing compilation target in artifact metadata")?;

    ensure!(
        !target.is_empty(),
        "empty compilation target in artifact metadata"
    );
    ensure!(
        target.len() == 1,
        "expected exactly one compilation target, found {}",
        target.len()
    );

    let (path, name) = target.iter().next().context("empty compilation target")?;
    ensure!(
        !path.is_empty() && !name.is_empty(),
        "empty path or contract name in compilation target"
    );

    Ok(BuildArtifactId {
        path: PathBuf::from(path),
        name: name.clone(),
    })
}

/// Find the named contract definition in the AST.
///
/// "Contract" here refers to the `ContractDefinition` AST node, which covers
/// contracts, interfaces, and libraries.
fn get_contract_definition<'a>(
    ast: &'a solc::ast::SourceUnit,
    contract_name: &str,
) -> Result<&'a solc::ast::ContractDefinition> {
    for node in &ast.nodes {
        if let solc::ast::SourceUnitNode::ContractDefinition(def) = node
            && def.name == contract_name
        {
            return Ok(def);
        }
    }
    bail!("contract `{}` not found in AST", contract_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // BuildArtifactId tests
    // -----------------------------------------------------------------------

    #[test]
    fn build_artifact_id_from_valid_string() {
        let id = BuildArtifactId::try_from("src/Counter.sol:Counter").unwrap();
        assert_eq!(id.path, PathBuf::from("src/Counter.sol"));
        assert_eq!(id.name, "Counter");
    }

    #[test]
    fn build_artifact_id_from_string_owned() {
        let id = BuildArtifactId::try_from("src/Counter.sol:Counter".to_owned()).unwrap();
        assert_eq!(id.path, PathBuf::from("src/Counter.sol"));
        assert_eq!(id.name, "Counter");
    }

    #[test]
    fn build_artifact_id_from_str() {
        let id = BuildArtifactId::try_from("src/Counter.sol:Counter").unwrap();
        assert_eq!(id.path, PathBuf::from("src/Counter.sol"));
        assert_eq!(id.name, "Counter");
    }

    #[test]
    fn build_artifact_id_display() {
        let id = BuildArtifactId::try_from("src/Counter.sol:Counter").unwrap();
        assert_eq!(id.to_string(), "src/Counter.sol:Counter");
    }

    #[test]
    fn build_artifact_id_from_str_multiple_colons() {
        // splitn(2, ':') uses the first colon as the separator
        let id = BuildArtifactId::try_from("src/a:b/Counter.sol:Counter").unwrap();
        assert_eq!(id.path, PathBuf::from("src/a"));
        assert_eq!(id.name, "b/Counter.sol:Counter");
    }

    #[test]
    fn build_artifact_id_missing_colon_fails() {
        let err = BuildArtifactId::try_from("src/Counter.sol").unwrap_err();
        assert!(err.to_string().contains("invalid build artifact id"));
    }

    #[test]
    fn build_artifact_id_empty_path_fails() {
        let err = BuildArtifactId::try_from(":Counter").unwrap_err();
        assert!(err.to_string().contains("non-empty"));
    }

    #[test]
    fn build_artifact_id_empty_name_fails() {
        let err = BuildArtifactId::try_from("src/Counter.sol:").unwrap_err();
        assert!(err.to_string().contains("non-empty"));
    }

    #[test]
    fn build_artifact_id_empty_string_fails() {
        let err = BuildArtifactId::try_from("").unwrap_err();
        assert!(err.to_string().contains("invalid build artifact id"));
    }

    // -----------------------------------------------------------------------
    // BuildArtifact synthetic parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_artifact_missing_metadata_fails() {
        let json = r#"{"abi":[],"bytecode":{"object":"","sourceMap":""},"deployedBytecode":{"object":"","sourceMap":""},"ast":{"id":0,"absolutePath":"","exportedSymbols":{},"src":"0:0:0","nodes":[]}}"#;
        let err = BuildArtifact::from_json_str(json).unwrap_err();
        assert!(err.to_string().contains("missing compilation target"));
    }

    #[test]
    fn parse_artifact_contract_not_in_ast_fails() {
        let json = r#"{
            "abi": [],
            "bytecode": {"object": "", "sourceMap": ""},
            "deployedBytecode": {"object": "", "sourceMap": ""},
            "ast": {
                "id": 0,
                "absolutePath": "src/Foo.sol",
                "exportedSymbols": {},
                "src": "0:0:0",
                "nodes": [
                    {
                        "nodeType": "ContractDefinition",
                        "id": 1,
                        "name": "Foo",
                        "abstract": false,
                        "contractKind": "contract",
                        "baseContracts": [],
                        "canonicalName": "Foo",
                        "fullyImplemented": true,
                        "linearizedBaseContracts": [],
                        "nodes": [],
                        "scope": 0,
                        "src": "0:0:0",
                        "contractDependencies": [],
                        "nameLocation": "",
                        "usedErrors": []
                    }
                ],
                "license": null
            },
            "metadata": {
                "settings": {
                    "compilationTarget": {
                        "src/Foo.sol": "Bar"
                    }
                }
            }
        }"#;
        let err = BuildArtifact::from_json_str(json).unwrap_err();
        assert!(err.to_string().contains("not found in AST"));
    }

    #[test]
    fn parse_artifact_invalid_json_fails() {
        let err = BuildArtifact::from_json_str("not json").unwrap_err();
        assert!(err.to_string().contains("expected ident"));
    }
}
