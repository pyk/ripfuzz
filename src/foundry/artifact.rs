//! Foundry artifact types and parsing.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, keccak256};
use anyhow::{Context, Result, bail, ensure};
use revm::primitives::Bytes;
use serde::Deserialize;
use tracing::{debug, instrument};

// ---------------------------------------------------------------------------
// Foundry's Artifact ID
// ---------------------------------------------------------------------------

/// Unique identifier for a compiled build artifact.
///
/// Format: `path:name` (e.g. `src/Counter.sol:Counter`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArtifactId {
    pub path: PathBuf,
    pub name: String,
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.name)
    }
}

impl From<ArtifactId> for String {
    fn from(id: ArtifactId) -> Self {
        format!("{}:{}", id.path.display(), id.name)
    }
}

impl From<&ArtifactId> for String {
    fn from(id: &ArtifactId) -> Self {
        format!("{}:{}", id.path.display(), id.name)
    }
}

impl TryFrom<String> for ArtifactId {
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
        ensure!(
            path.extension().is_some_and(|ext| ext == "sol"),
            "invalid build artifact id: path must end with `.sol`, got `{}`",
            path.display()
        );
        Ok(Self { path, name })
    }
}

impl TryFrom<&str> for ArtifactId {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl FromStr for ArtifactId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

// ---------------------------------------------------------------------------
// Foundry's Artifact
// ---------------------------------------------------------------------------

/// A compiled Solidity artifact loaded from a Foundry project.
#[derive(Clone, Debug, PartialEq)]
pub enum Artifact {
    Contract(ContractArtifact),
    Interface(InterfaceArtifact),
    Library(LibraryArtifact),
    Abstract(AbstractArtifact),
}

/// A concrete contract artifact with all data required for fuzzing.
#[derive(Clone, Debug, PartialEq)]
pub struct ContractArtifact {
    pub id: ArtifactId,
    pub project_path: PathBuf,
    pub ast: solc::ast::SourceUnit,
    pub abi: JsonAbi,
    pub bytecode: ArtifactBytecode,
    pub deployed_bytecode: ArtifactBytecode,
    pub storage_layout: Option<StorageLayout>,
    /// The numeric source ID assigned by the Solidity compiler for this
    /// artifact's source file within its compilation unit.
    pub source_id: usize,
}

impl ContractArtifact {
    /// Link the contract's bytecode with the given library addresses.
    pub fn link(&mut self, libs: &HashMap<String, Address>) {
        self.bytecode.link(libs);
        self.deployed_bytecode.link(libs);
    }
}

/// An interface artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceArtifact {
    pub id: ArtifactId,
    pub project_path: PathBuf,
    pub ast: solc::ast::SourceUnit,
    pub abi: JsonAbi,
    /// The numeric source ID assigned by the Solidity compiler for this
    /// artifact's source file within its compilation unit.
    pub source_id: usize,
}

/// A library artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryArtifact {
    pub id: ArtifactId,
    pub project_path: PathBuf,
    pub ast: solc::ast::SourceUnit,
    pub abi: JsonAbi,
    pub bytecode: ArtifactBytecode,
    pub deployed_bytecode: ArtifactBytecode,
    pub storage_layout: Option<StorageLayout>,
    /// The numeric source ID assigned by the Solidity compiler for this
    /// artifact's source file within its compilation unit.
    pub source_id: usize,
}

/// An abstract contract artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct AbstractArtifact {
    pub id: ArtifactId,
    pub project_path: PathBuf,
    pub ast: solc::ast::SourceUnit,
    pub abi: JsonAbi,
    /// The numeric source ID assigned by the Solidity compiler for this
    /// artifact's source file within its compilation unit.
    pub source_id: usize,
}

/// A single storage slot entry from the Solidity `storageLayout` output.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct StorageSlot {
    #[serde(rename = "astId")]
    pub ast_id: u64,
    pub contract: String,
    pub label: String,
    pub offset: u64,
    pub slot: String,
    #[serde(rename = "type")]
    pub type_name: String,
}

/// Type information for a single entry in the `storageLayout` `types` map.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct StorageTypeInfo {
    pub encoding: String,
    pub label: String,
    #[serde(rename = "numberOfBytes")]
    pub number_of_bytes: String,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub members: Vec<StructMember>,
}

/// A single member field of a struct type in the `storageLayout` output.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct StructMember {
    pub label: String,
    pub offset: u64,
    pub slot: String,
    #[serde(rename = "type")]
    pub type_name: String,
}

/// The `storageLayout` section of a compiled artifact.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct StorageLayout {
    pub storage: Vec<StorageSlot>,
    #[serde(default)]
    pub types: HashMap<String, StorageTypeInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct ArtifactJson {
    abi: JsonAbi,
    bytecode: ArtifactBytecode,
    #[serde(rename = "deployedBytecode")]
    deployed_bytecode: ArtifactBytecode,
    ast: solc::ast::SourceUnit,
    metadata: Option<ArtifactMetadata>,
    #[serde(rename = "storageLayout")]
    storage_layout: Option<StorageLayout>,
    #[serde(default)]
    id: usize,
}

/// A single link reference location within bytecode.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LinkReference {
    pub start: usize,
    pub length: usize,
}

/// Link references grouped by source file and library name.
///
/// Outer key: source file path. Inner key: library name. Value: list of
/// placeholder locations in the bytecode object.
pub type LinkReferences = HashMap<String, HashMap<String, Vec<LinkReference>>>;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ArtifactBytecode {
    #[serde(default)]
    pub object: String,
    #[serde(default, rename = "sourceMap")]
    pub source_map: String,
    #[serde(default, rename = "linkReferences")]
    pub link_references: LinkReferences,
}

impl ArtifactBytecode {
    /// Return true if the bytecode object contains unlinked library placeholders.
    pub fn is_unlinked(&self) -> bool {
        self.object.contains("__$")
    }

    /// Return the library dependencies declared in `link_references`.
    ///
    /// The returned map maps a source file path to a list of library names.
    pub fn library_dependencies(&self) -> HashMap<String, Vec<String>> {
        let mut deps = HashMap::new();
        let refs = self.link_references.clone();
        for (file, libs) in refs {
            let names: Vec<String> = libs.into_keys().collect();
            deps.insert(file, names);
        }
        deps
    }

    /// Link the bytecode by replacing all library placeholders with the given addresses.
    ///
    /// `libs` maps a fully-qualified library identifier (`path:name`) to its deployed address.
    pub fn link(&mut self, libs: &HashMap<String, Address>) {
        for (identifier, address) in libs {
            let placeholder = Self::placeholder_for(identifier);
            let address_hex = hex::encode(address);
            self.object = self.object.replace(&placeholder, &address_hex);
        }
    }

    /// Compute the Solidity placeholder string for a library identifier.
    ///
    /// The placeholder format is `__$<keccak256(identifier)[:34]>$__`.
    pub fn placeholder_for(identifier: &str) -> String {
        let hash = keccak256(identifier.as_bytes());
        let hex = alloy_primitives::hex::encode(hash);
        format!("__${}$__", &hex[..34])
    }

    /// Parse the linked bytecode object as hex bytes.
    ///
    /// Returns an empty `Bytes` if the object is still unlinked or empty.
    pub fn to_bytes(&self) -> Bytes {
        if self.is_unlinked() {
            return Bytes::new();
        }
        self.object.parse().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ArtifactMetadata {
    settings: Option<ArtifactSettings>,
}

#[derive(Debug, Clone, Deserialize)]
struct ArtifactSettings {
    #[serde(rename = "compilationTarget")]
    compilation_target: Option<HashMap<String, String>>,
}

impl Artifact {
    /// Link the artifact's bytecode with the given library addresses.
    ///
    /// Only affects contract and library artifacts.
    pub fn link(&mut self, libs: &HashMap<String, Address>) {
        match self {
            Self::Contract(a) => a.link(libs),
            Self::Library(a) => {
                a.bytecode.link(libs);
                a.deployed_bytecode.link(libs);
            }
            _ => {}
        }
    }

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
        let json: ArtifactJson = serde_json::from_str(content)?;

        let id = get_artifact_id(&json)?;

        let def = get_contract_definition(&json.ast, &id.name)?;
        let source_id = json.id;
        Ok(match def.contract_kind {
            solc::ast::ContractKind::Contract if !def.r#abstract => {
                Self::Contract(ContractArtifact {
                    id,
                    project_path: PathBuf::new(),
                    ast: json.ast,
                    abi: json.abi,
                    bytecode: json.bytecode,
                    deployed_bytecode: json.deployed_bytecode,
                    storage_layout: json.storage_layout,
                    source_id,
                })
            }
            solc::ast::ContractKind::Contract => Self::Abstract(AbstractArtifact {
                id,
                project_path: PathBuf::new(),
                ast: json.ast,
                abi: json.abi,
                source_id,
            }),
            solc::ast::ContractKind::Interface => Self::Interface(InterfaceArtifact {
                id,
                project_path: PathBuf::new(),
                ast: json.ast,
                abi: json.abi,
                source_id,
            }),
            solc::ast::ContractKind::Library => Self::Library(LibraryArtifact {
                id,
                project_path: PathBuf::new(),
                ast: json.ast,
                abi: json.abi,
                bytecode: json.bytecode,
                deployed_bytecode: json.deployed_bytecode,
                storage_layout: json.storage_layout,
                source_id,
            }),
        })
    }

    /// The unique identifier of this artifact.
    pub fn id(&self) -> &ArtifactId {
        match self {
            Self::Contract(a) => &a.id,
            Self::Interface(a) => &a.id,
            Self::Library(a) => &a.id,
            Self::Abstract(a) => &a.id,
        }
    }

    /// The numeric source ID assigned by the Solidity compiler for this
    /// artifact's source file within its compilation unit.
    pub fn source_id(&self) -> usize {
        match self {
            Self::Contract(a) => a.source_id,
            Self::Interface(a) => a.source_id,
            Self::Library(a) => a.source_id,
            Self::Abstract(a) => a.source_id,
        }
    }

    /// The absolute path to the project this artifact was built from.
    pub fn project_path(&self) -> &Path {
        match self {
            Self::Contract(a) => &a.project_path,
            Self::Interface(a) => &a.project_path,
            Self::Library(a) => &a.project_path,
            Self::Abstract(a) => &a.project_path,
        }
    }

    /// Set the project path for this artifact.
    pub fn set_project_path(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref().to_path_buf();
        match self {
            Self::Contract(a) => a.project_path = path,
            Self::Interface(a) => a.project_path = path,
            Self::Library(a) => a.project_path = path,
            Self::Abstract(a) => a.project_path = path,
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

    /// Consume the artifact and return its JSON ABI.
    pub fn into_abi(self) -> JsonAbi {
        match self {
            Self::Contract(a) => a.abi,
            Self::Interface(a) => a.abi,
            Self::Library(a) => a.abi,
            Self::Abstract(a) => a.abi,
        }
    }

    /// The deployment bytecode (initcode), if the artifact has any.
    pub fn bytecode(&self) -> Option<&ArtifactBytecode> {
        match self {
            Self::Contract(a) => Some(&a.bytecode),
            Self::Library(a) => Some(&a.bytecode),
            _ => None,
        }
    }

    /// The deployed bytecode, if the artifact has any.
    pub fn deployed_bytecode(&self) -> Option<&ArtifactBytecode> {
        match self {
            Self::Contract(a) => Some(&a.deployed_bytecode),
            Self::Library(a) => Some(&a.deployed_bytecode),
            _ => None,
        }
    }

    /// The storage layout, if the artifact has any.
    pub fn storage_layout(&self) -> Option<&StorageLayout> {
        match self {
            Self::Contract(a) => a.storage_layout.as_ref(),
            Self::Library(a) => a.storage_layout.as_ref(),
            _ => None,
        }
    }
}

/// Extract `ArtifactId` from the artifact metadata.
fn get_artifact_id(json: &ArtifactJson) -> Result<ArtifactId> {
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

    Ok(ArtifactId {
        path: PathBuf::from(path),
        name: name.clone(),
    })
}

/// Find the named contract definition in the AST.
///
/// "Contract" here refers to the `ContractDefinition` AST node, which covers
/// contracts, interfaces, and libraries.
pub fn get_contract_definition<'a>(
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
    // ArtifactId tests
    // -----------------------------------------------------------------------

    #[test]
    fn artifact_id_from_valid_string() {
        let id = ArtifactId::try_from("src/Counter.sol:Counter").unwrap();
        assert_eq!(id.path, PathBuf::from("src/Counter.sol"));
        assert_eq!(id.name, "Counter");
    }

    #[test]
    fn artifact_id_from_string_owned() {
        let id = ArtifactId::try_from("src/Counter.sol:Counter".to_owned()).unwrap();
        assert_eq!(id.path, PathBuf::from("src/Counter.sol"));
        assert_eq!(id.name, "Counter");
    }

    #[test]
    fn artifact_id_from_str() {
        let id = ArtifactId::try_from("src/Counter.sol:Counter").unwrap();
        assert_eq!(id.path, PathBuf::from("src/Counter.sol"));
        assert_eq!(id.name, "Counter");
    }

    #[test]
    fn artifact_id_display() {
        let id = ArtifactId::try_from("src/Counter.sol:Counter").unwrap();
        assert_eq!(id.to_string(), "src/Counter.sol:Counter");
    }

    #[test]
    fn artifact_id_from_str_multiple_colons() {
        // splitn(2, ':') uses the first colon as the separator
        let id = ArtifactId::try_from("src/a.sol:b/Counter.sol:Counter").unwrap();
        assert_eq!(id.path, PathBuf::from("src/a.sol"));
        assert_eq!(id.name, "b/Counter.sol:Counter");
    }

    #[test]
    fn artifact_id_missing_colon_fails() {
        let err = ArtifactId::try_from("src/Counter.sol").unwrap_err();
        assert!(err.to_string().contains("invalid build artifact id"));
    }

    #[test]
    fn artifact_id_empty_path_fails() {
        let err = ArtifactId::try_from(":Counter").unwrap_err();
        assert!(err.to_string().contains("non-empty"));
    }

    #[test]
    fn artifact_id_empty_name_fails() {
        let err = ArtifactId::try_from("src/Counter.sol:").unwrap_err();
        assert!(err.to_string().contains("non-empty"));
    }

    #[test]
    fn artifact_id_empty_string_fails() {
        let err = ArtifactId::try_from("").unwrap_err();
        assert!(err.to_string().contains("invalid build artifact id"));
    }

    #[test]
    fn artifact_id_path_without_sol_extension_fails() {
        let err = ArtifactId::try_from("test/ImpossibleBug:ImpossibleBug").unwrap_err();
        assert!(err.to_string().contains("must end with `.sol`"));
    }

    // -----------------------------------------------------------------------
    // Artifact synthetic parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_artifact_missing_metadata_fails() {
        let json = r#"{"abi":[],"bytecode":{"object":"","sourceMap":""},"deployedBytecode":{"object":"","sourceMap":""},"ast":{"id":0,"absolutePath":"","exportedSymbols":{},"src":"0:0:0","nodes":[]}}"#;
        let err = Artifact::from_json_str(json).unwrap_err();
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
        let err = Artifact::from_json_str(json).unwrap_err();
        assert!(err.to_string().contains("not found in AST"));
    }

    #[test]
    fn parse_artifact_invalid_json_fails() {
        let err = Artifact::from_json_str("not json").unwrap_err();
        assert!(err.to_string().contains("expected ident"));
    }
}
