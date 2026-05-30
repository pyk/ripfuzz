//! Trace context for formatting and decoding raw execution traces.
//!
//! [`TraceContext`] collects ABIs and address labels from build artifacts,
//! then provides lookup methods used by [`Trace::display_with`](crate::evm::trace::Trace::display_with).

use std::collections::HashMap;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{B256, FixedBytes, U256, keccak256};
use alloy_sol_types::SolError;
use anyhow::Result;
use revm::primitives::{Address, Bytes};

use crate::evm::cheatcode::VM_ADDRESS;
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
/// Metadata for an array variable so that hashed element slots can be
/// resolved back to `array[index]`.
#[derive(Debug, Clone)]
struct ArrayInfo {
    name: String,
    element_type: super::StorageType,
    element_slots: usize,
    start_slot: U256,
    /// Fixed length for fixed arrays; `None` for dynamic arrays.
    len: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct TraceContext {
    labels: HashMap<Address, String>,
    abis: Vec<JsonAbi>,
    bytecode_entries: Vec<BytecodeEntry>,
    /// Maps contract name -> (slot -> (label, storage_type)).
    storage_names: HashMap<String, HashMap<U256, (String, super::StorageType)>>,
    /// Maps contract name -> list of arrays for element-slot resolution.
    array_info: HashMap<String, Vec<ArrayInfo>>,
}

impl Default for TraceContext {
    fn default() -> Self {
        let mut labels = HashMap::new();
        labels.insert(VM_ADDRESS, "RaptorVm".into());
        Self {
            labels,
            abis: Vec::new(),
            bytecode_entries: Vec::new(),
            storage_names: HashMap::new(),
            array_info: HashMap::new(),
        }
    }
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
        let mut ctx = Self::default();
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
            if let Some((names, arrays)) = parse_storage_layout(&artifact) {
                ctx.storage_names.insert(artifact.name().into(), names);
                ctx.array_info.insert(artifact.name().into(), arrays);
            }
            ctx.abis.push(artifact.into_abi());
        }
        ctx.bytecode_entries = bytecode_entries;
        ctx
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

    /// Look up the human-readable name for a storage slot in a contract.
    ///
    /// The `contract_name` is the label or name that was registered for the
    /// contract address (e.g. via bytecode matching or explicit `with_label`).
    pub fn resolve_storage_name(&self, contract_name: &str, slot: &U256) -> Option<String> {
        if let Some((label, ty)) = self
            .storage_names
            .get(contract_name)
            .and_then(|map| map.get(slot))
        {
            return match ty {
                // Fixed array: base slot is the first element.
                super::StorageType::Array { len: Some(_), .. } => Some(format!("{label}[0]")),
                // Dynamic array: base slot holds the length.
                super::StorageType::Array { len: None, .. } => Some(format!("{label}.length")),
                _ => Some(label.clone()),
            };
        }
        // Check if the slot belongs to an array element.
        // Pick the array with the largest start_slot that is <= the target
        // slot (most specific match).
        if let Some(arrays) = self.array_info.get(contract_name) {
            let mut best: Option<&ArrayInfo> = None;
            for array in arrays {
                if *slot >= array.start_slot {
                    let offset = slot - array.start_slot;
                    let index = offset / U256::from(array.element_slots);
                    let in_bounds = match array.len {
                        Some(len) => match usize::try_from(index) {
                            Ok(idx) => idx < len,
                            Err(_) => false,
                        },
                        None => true,
                    };
                    if !in_bounds {
                        continue;
                    }
                    if best
                        .map(|b| array.start_slot > b.start_slot)
                        .unwrap_or(true)
                    {
                        best = Some(array);
                    }
                }
            }
            if let Some(array) = best {
                let offset = slot - array.start_slot;
                let index = offset / U256::from(array.element_slots);
                return Some(format!("{}[{}]", array.name, index));
            }
        }
        None
    }

    /// Look up the storage type for a storage slot in a contract.
    pub fn resolve_storage_type(
        &self,
        contract_name: &str,
        slot: &U256,
    ) -> Option<&super::StorageType> {
        if let Some((_, ty)) = self
            .storage_names
            .get(contract_name)
            .and_then(|map| map.get(slot))
        {
            return match ty {
                // Fixed array: base slot is the first element.
                super::StorageType::Array {
                    element,
                    len: Some(_),
                    ..
                } => Some(element),
                _ => Some(ty),
            };
        }
        // Check if the slot belongs to an array element.
        // Pick the array with the largest start_slot that is <= the target
        // slot (most specific match).
        if let Some(arrays) = self.array_info.get(contract_name) {
            let mut best: Option<&ArrayInfo> = None;
            for array in arrays {
                if *slot >= array.start_slot {
                    let offset = slot - array.start_slot;
                    let index = offset / U256::from(array.element_slots);
                    let in_bounds = match array.len {
                        Some(len) => match usize::try_from(index) {
                            Ok(idx) => idx < len,
                            Err(_) => false,
                        },
                        None => true,
                    };
                    if !in_bounds {
                        continue;
                    }
                    if best
                        .map(|b| array.start_slot > b.start_slot)
                        .unwrap_or(true)
                    {
                        best = Some(array);
                    }
                }
            }
            if let Some(array) = best {
                return Some(&array.element_type);
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

type StorageLayoutResult = (HashMap<U256, (String, super::StorageType)>, Vec<ArrayInfo>);

/// Parse state-variable names, types, and array metadata from an artifact's
/// `storageLayout` output.
fn parse_storage_layout(artifact: &Artifact) -> Option<StorageLayoutResult> {
    let layout = artifact.storage_layout()?;
    let mut names = HashMap::new();
    let mut arrays = Vec::new();
    for entry in &layout.storage {
        let slot = entry.slot.parse::<U256>().ok()?;
        let ty = super::StorageType::parse(&entry.type_name)?;
        // checkrs: allow(clone_in_loops)
        names.insert(slot, (entry.label.clone(), ty.clone()));

        // Build array metadata for element-slot resolution.
        // checkrs: allow(nested_if_let)
        if let super::StorageType::Array {
            element,
            len: array_len,
        } = &ty
        {
            let start_slot = if array_len.is_some() {
                // Fixed array: elements start at the base slot.
                slot
            } else {
                // Dynamic array: elements start at keccak256(base_slot).
                let mut base_bytes = [0u8; 32];
                base_bytes.copy_from_slice(&slot.to_be_bytes::<32>());
                U256::from_be_bytes(keccak256(base_bytes).0)
            };

            let element_slots = element_byte_slots(&layout.types, &entry.type_name);
            arrays.push(ArrayInfo {
                // checkrs: allow(clone_in_loops)
                name: entry.label.clone(),
                // checkrs: allow(clone_in_loops)
                element_type: *element.clone(),
                element_slots,
                start_slot,
                len: *array_len,
            });
        }
    }
    arrays.sort_by(|a, b| b.start_slot.cmp(&a.start_slot));
    Some((names, arrays)).filter(|(n, _)| !n.is_empty())
}

/// Compute how many 32-byte slots a single array element occupies.
///
/// Looks up the array's `base` type in the `storageLayout` types map and
/// uses the base type's `numberOfBytes`.
fn element_byte_slots(
    types: &HashMap<String, crate::foundry::StorageTypeInfo>,
    array_type_name: &str,
) -> usize {
    let info = types.get(array_type_name);
    let base_type = info.and_then(|t| t.base.as_ref());
    let bytes = base_type
        .and_then(|base| types.get(base))
        .and_then(|t| t.number_of_bytes.parse::<usize>().ok())
        .or_else(|| info.and_then(|t| t.number_of_bytes.parse::<usize>().ok()))
        .unwrap_or(32);
    bytes.div_ceil(32)
}

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

    use crate::evm::cheatcode::VM_ADDRESS;
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

    #[test]
    fn vm_address_is_labeled_by_default() {
        let ctx = TraceContext::new();
        assert_eq!(ctx.get_label(&VM_ADDRESS), Some("RaptorVm"));

        let ctx = TraceContext::from_project(&Project::new("fixtures/trace-context")).unwrap();
        assert_eq!(ctx.get_label(&VM_ADDRESS), Some("RaptorVm"));
    }
}
