//! Trace context for formatting and decoding raw execution traces.
//!
//! [`TraceContext`] collects ABIs and address labels from build artifacts,
//! then provides lookup methods used by [`Trace::display_with`](crate::evm::trace::Trace::display_with).

use std::collections::HashMap;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{B256, FixedBytes, keccak256};
use alloy_sol_types::SolError;
use anyhow::Result;
use revm::primitives::{Address, Bytes};

use crate::foundry::{Artifact, ArtifactId, Project};

/// A single bytecode entry for matching runtime code against artifacts.
#[derive(Debug, Clone)]
struct BytecodeEntry {
    name: String,
    base_hash: B256,
    positions: Vec<(usize, usize)>,
}

/// Context for decoding and formatting a raw [`Trace`](crate::evm::trace::Trace).
///
/// Collects ABIs, address labels, and runtime bytecode hashes from build
/// artifacts, then provides lookup methods for the trace display logic.
#[derive(Debug, Clone, Default)]
pub struct TraceContext {
    labels: HashMap<Address, String>,
    abis: Vec<JsonAbi>,
    bytecode_entries: Vec<BytecodeEntry>,
}

impl TraceContext {
    /// Create an empty [`TraceContext`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a [`TraceContext`] from all build artifacts in a [`Project`].
    pub fn from_project(project: &Project) -> Result<Self> {
        let artifacts = project.load_artifacts()?;
        Ok(Self::from_artifacts(artifacts))
    }

    /// Build a [`TraceContext`] from a map of build artifacts.
    pub fn from_artifacts(artifacts: HashMap<ArtifactId, Artifact>) -> Self {
        let mut abis = Vec::with_capacity(artifacts.len());
        let mut bytecode_entries = Vec::new();
        for artifact in artifacts.into_values() {
            let bytecode = artifact.deployed_bytecode();
            let code =
                bytecode.map(|b| parse_bytecode_with_placeholders(&b.object, &b.link_references));
            if let Some(code) = code
                && !code.is_empty()
            {
                let positions = bytecode
                    .map(|b| collect_link_positions(&b.link_references))
                    .unwrap_or_default();
                let mut masked = code;
                zero_out_positions(&mut masked, &positions);
                bytecode_entries.push(BytecodeEntry {
                    name: artifact.name().into(),
                    base_hash: keccak256(&masked),
                    positions,
                });
            }
            abis.push(artifact.into_abi());
        }
        Self {
            labels: HashMap::new(),
            abis,
            bytecode_entries,
        }
    }

    /// Set an address label for formatting.
    pub fn with_label(mut self, address: Address, label: impl Into<String>) -> Self {
        self.labels.insert(address, label.into());
        self
    }

    /// Add an ABI for decoding function calls and revert reasons.
    pub fn with_abi(mut self, abi: JsonAbi) -> Self {
        self.abis.push(abi);
        self
    }

    /// Get the label for an address, if one is registered.
    pub fn get_label(&self, address: &Address) -> Option<&str> {
        self.labels.get(address).map(|s| s.as_str())
    }

    /// Find the first ABI that contains a function matching the given selector.
    pub fn get_abi(&self, selector: [u8; 4]) -> Option<&JsonAbi> {
        let sel = FixedBytes::new(selector);
        self.abis
            .iter()
            .find(|abi| abi.function_by_selector(sel).is_some())
    }

    /// Look up a contract name by its runtime bytecode.
    ///
    /// Matches against the artifact bytecode index, masking out library
    /// link-reference positions so that linked and unlinked bytecodes match.
    pub fn resolve_by_bytecode(&self, code: &Bytes) -> Option<&str> {
        let mut masked = code.to_vec();
        for entry in &self.bytecode_entries {
            zero_out_positions(&mut masked, &entry.positions);
            if keccak256(&masked) == entry.base_hash {
                return Some(&entry.name);
            }
            // Restore masked bytes for the next entry.
            for (start, len) in &entry.positions {
                for i in *start..*start + *len {
                    if i < masked.len() {
                        masked[i] = code[i];
                    }
                }
            }
        }
        None
    }

    /// Decode a function call from its input data.
    ///
    /// Returns the function name (if found) and a formatted argument string.
    pub fn decode_call(&self, data: &Bytes) -> (Option<&str>, String) {
        if data.len() < 4 {
            return (None, String::new());
        }
        let sel: [u8; 4] = data[..4].try_into().unwrap_or_default();
        for abi in &self.abis {
            if let Some(func) = abi.function_by_selector(FixedBytes::new(sel)) {
                let types: Vec<DynSolType> = func
                    .inputs
                    .iter()
                    .filter_map(|p| DynSolType::parse(&p.selector_type()).ok())
                    .collect();
                let args = if types.is_empty() {
                    String::new()
                } else {
                    let tuple = DynSolType::Tuple(types);
                    match tuple.abi_decode_params(&data[4..]) {
                        Ok(DynSolValue::Tuple(values)) => format_args(&values),
                        Ok(other) => format_args(&[other]),
                        Err(_) => "...".into(),
                    }
                };
                return (Some(func.name.as_str()), args);
            }
        }
        (None, "...".into())
    }

    /// Decode a revert reason from its output data.
    pub fn decode_revert(&self, data: &Bytes) -> String {
        if data.is_empty() {
            return "reverted".into();
        }

        // Try generic Error(string) first
        if data.len() >= 4 {
            let sel: [u8; 4] = data[..4].try_into().unwrap_or_default();
            if sel == [0x08, 0xc3, 0x79, 0xa0]
                && let Ok(revert) = alloy_sol_types::Revert::abi_decode(data)
            {
                return revert.reason;
            }
        }

        // Try Solidity panic
        if data.len() >= 4 {
            let sel: [u8; 4] = data[..4].try_into().unwrap_or_default();
            if sel == [0x4e, 0x48, 0x7b, 0x71]
                && let Ok(panic) = alloy_sol_types::Panic::abi_decode(data)
            {
                return panic.as_geth_str().into();
            }
        }

        // Try ABI errors
        if data.len() >= 4 {
            let sel: FixedBytes<4> = FixedBytes::new(data[..4].try_into().unwrap_or_default());
            for abi in &self.abis {
                for error in abi.errors() {
                    if error.selector().as_slice() == sel.as_slice() {
                        if error.inputs.is_empty() {
                            return format!("{}()", error.name);
                        } else {
                            return format!("{}(...)", error.name);
                        }
                    }
                }
            }
        }

        format!("0x{}", hex::encode(data))
    }
}

pub(super) fn format_value(v: &DynSolValue) -> String {
    match v {
        DynSolValue::Bool(b) => format!("{b}"),
        DynSolValue::Uint(n, _) => format!("{n}"),
        DynSolValue::Int(n, _) => format!("{n}"),
        DynSolValue::Address(a) => format!("{a}"),
        DynSolValue::String(s) => s.into(),
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        DynSolValue::Array(arr) | DynSolValue::FixedArray(arr) => {
            let inner: Vec<String> = arr.iter().map(format_value).collect();
            format!("[{inner}]", inner = inner.join(", "))
        }
        DynSolValue::Tuple(vals) => {
            let inner: Vec<String> = vals.iter().map(format_value).collect();
            format!("({inner})", inner = inner.join(", "))
        }
        _ => format!("{v:?}"),
    }
}

pub(super) fn format_args(values: &[DynSolValue]) -> String {
    if values.is_empty() {
        return String::new();
    }
    values
        .iter()
        .map(format_value)
        .collect::<Vec<String>>()
        .join(", ")
}

use crate::foundry::LinkReferences;

/// Collect all link-reference positions from a [`LinkReferences`] map.
fn collect_link_positions(link_refs: &LinkReferences) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for libs in link_refs.values() {
        for refs in libs.values() {
            for r in refs {
                out.push((r.start, r.length));
            }
        }
    }
    out
}

/// Zero out the bytes at the given positions in a mutable buffer.
fn zero_out_positions(buf: &mut [u8], positions: &[(usize, usize)]) {
    for (start, len) in positions {
        for i in *start..*start + *len {
            if i < buf.len() {
                buf[i] = 0;
            }
        }
    }
}

/// Parse a bytecode object string, replacing library placeholders at the
/// given link-reference positions with zero bytes.
fn parse_bytecode_with_placeholders(object: &str, link_refs: &LinkReferences) -> Vec<u8> {
    let hex = object.strip_prefix("0x").unwrap_or(object);

    let mut hex_positions = Vec::new();
    for libs in link_refs.values() {
        for refs in libs.values() {
            for r in refs {
                hex_positions.push((r.start * 2, r.length * 2));
            }
        }
    }
    hex_positions.sort_by_key(|(start, _)| *start);

    let mut cleaned = String::new();
    let mut last_end = 0;
    for (start, len) in hex_positions {
        cleaned.push_str(&hex[last_end..start]);
        cleaned.push_str(&"00".repeat(len / 2));
        last_end = start + len;
    }
    cleaned.push_str(&hex[last_end..]);

    hex::decode(cleaned).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use alloy_primitives::Address;
    use revm::primitives::Bytes;

    use crate::foundry::{Artifact, ArtifactId, Project};

    use super::TraceContext;

    /// Load a single artifact from a fixture project by its id string.
    fn load_artifact(path: impl AsRef<Path>, id: &str) -> Artifact {
        let project = Project::new(path);
        let artifacts = project.load_artifacts().unwrap();
        artifacts
            .get(&ArtifactId::try_from(id).unwrap())
            .unwrap()
            .clone()
    }

    /// Link a copy of the given artifact with a dummy library address.
    fn linked_artifact(mut artifact: Artifact) -> Artifact {
        let mut libs = HashMap::new();
        libs.insert("src/MathLib.sol:MathLib".into(), Address::repeat_byte(0xab));
        artifact.link(&libs);
        artifact
    }

    #[test]
    fn linked_bytecode_matches_unlinked_artifact() {
        let unlinked = load_artifact(
            "fixtures/trace-context",
            "src/LinkedCounter.sol:LinkedCounter",
        );
        let linked = linked_artifact(unlinked.clone());

        let ctx = TraceContext::from_artifacts({
            let mut map = HashMap::new();
            map.insert(unlinked.id().clone(), unlinked);
            map
        });

        let runtime = linked.deployed_bytecode().unwrap().to_bytes();
        assert!(
            !runtime.is_empty(),
            "runtime bytecode must not be empty after linking"
        );

        let name = ctx.resolve_by_bytecode(&runtime);
        assert_eq!(
            name,
            Some("LinkedCounter"),
            "linked runtime must match unlinked artifact"
        );
    }

    #[test]
    fn exact_bytecode_matches_linked_artifact() {
        let linked = linked_artifact(load_artifact(
            "fixtures/trace-context",
            "src/LinkedCounter.sol:LinkedCounter",
        ));

        let ctx = TraceContext::from_artifacts({
            let mut map = HashMap::new();
            map.insert(linked.id().clone(), linked.clone());
            map
        });

        let runtime = linked.deployed_bytecode().unwrap().to_bytes();
        assert!(!runtime.is_empty());

        let name = ctx.resolve_by_bytecode(&runtime);
        assert_eq!(
            name,
            Some("LinkedCounter"),
            "exact linked bytecode must match itself"
        );
    }

    #[test]
    fn unknown_bytecode_returns_none() {
        let unlinked = load_artifact(
            "fixtures/trace-context",
            "src/LinkedCounter.sol:LinkedCounter",
        );

        let ctx = TraceContext::from_artifacts({
            let mut map = HashMap::new();
            map.insert(unlinked.id().clone(), unlinked);
            map
        });

        let unknown = Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            ctx.resolve_by_bytecode(&unknown),
            None,
            "unknown bytecode must not match"
        );
    }
}
