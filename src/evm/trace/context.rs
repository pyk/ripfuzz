//! Trace context for formatting and decoding raw execution traces.
//!
//! [`TraceContext`] collects ABIs and address labels from build artifacts,
//! then provides lookup methods used by [`Trace::display_with`](crate::evm::trace::Trace::display_with).

use std::collections::HashMap;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::{JsonAbi, Param};
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
/// A single field within a struct element of an array.
#[derive(Debug, Clone)]
struct StructField {
    name: String,
    slot_offset: usize,
    ty: super::StorageType,
}

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
    /// Struct field layout when the element type is a struct.
    struct_fields: Option<Vec<StructField>>,
}

/// A single packed variable within a storage slot.
#[derive(Debug, Clone)]
struct StorageEntry {
    offset: usize,
    name: String,
    ty: super::StorageType,
    bytes: usize,
}

/// A single changed variable resolved from a packed storage slot.
#[derive(Debug)]
pub struct StorageChangeInfo<'a> {
    pub name: String,
    pub ty: &'a super::StorageType,
    pub offset: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone)]
pub struct TraceContext {
    labels: HashMap<Address, String>,
    abis: Vec<JsonAbi>,
    bytecode_entries: Vec<BytecodeEntry>,
    /// Maps contract name -> (slot -> list of packed variables).
    storage_names: HashMap<String, HashMap<U256, Vec<StorageEntry>>>,
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
        if let Some(entries) = self
            .storage_names
            .get(contract_name)
            .and_then(|map| map.get(slot))
            && let Some(entry) = entries.first()
        {
            return match entry.ty {
                // Fixed array: base slot is the first element.
                super::StorageType::Array { len: Some(_), .. } => {
                    Some(format!("{}[0]", entry.name))
                }
                // Dynamic array: base slot holds the length.
                super::StorageType::Array { len: None, .. } => {
                    Some(format!("{}.length", entry.name))
                }
                _ => Some(entry.name.clone()),
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
                let field_offset =
                    u64::try_from(offset % U256::from(array.element_slots)).unwrap_or(0) as usize;
                let name = format!("{}[{}]", array.name, index);
                if let Some(fields) = &array.struct_fields
                    && let Some(field) = fields.iter().find(|f| f.slot_offset == field_offset)
                {
                    return Some(format!("{}.{}", name, field.name));
                }
                return Some(name);
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
        if let Some(entries) = self
            .storage_names
            .get(contract_name)
            .and_then(|map| map.get(slot))
            && let Some(entry) = entries.first()
        {
            return match &entry.ty {
                // Fixed array: base slot is the first element.
                super::StorageType::Array {
                    element,
                    len: Some(_),
                    ..
                } => Some(element.as_ref()),
                _ => Some(&entry.ty),
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
                let field_offset =
                    u64::try_from(offset % U256::from(array.element_slots)).unwrap_or(0) as usize;
                if let Some(fields) = &array.struct_fields
                    && let Some(field) = fields.iter().find(|f| f.slot_offset == field_offset)
                {
                    return Some(&field.ty);
                }
                return Some(&array.element_type);
            }
        }
        None
    }

    /// Resolve all packed variables that changed within a storage slot.
    ///
    /// Returns a list of [`StorageChangeInfo`] for every variable whose
    /// extracted value differs between `old_value` and `new_value`.
    pub fn resolve_storage_changes(
        &self,
        contract_name: &str,
        slot: &U256,
        old_value: U256,
        new_value: U256,
    ) -> Vec<StorageChangeInfo<'_>> {
        let mut changes = Vec::new();
        // checkrs: allow(nested_if_let)
        if let Some(entries) = self
            .storage_names
            .get(contract_name)
            .and_then(|map| map.get(slot))
        {
            for entry in entries {
                let old_extracted = if entry.offset == 0 && entry.bytes >= 32 {
                    old_value
                } else {
                    let shifted = old_value >> (entry.offset * 8);
                    let mask = (U256::from(1) << (entry.bytes * 8)) - U256::from(1);
                    shifted & mask
                };
                let new_extracted = if entry.offset == 0 && entry.bytes >= 32 {
                    new_value
                } else {
                    let shifted = new_value >> (entry.offset * 8);
                    let mask = (U256::from(1) << (entry.bytes * 8)) - U256::from(1);
                    shifted & mask
                };
                if old_extracted != new_extracted {
                    let name = match entry.ty {
                        super::StorageType::Array { len: Some(_), .. } => {
                            format!("{}[0]", entry.name)
                        }
                        super::StorageType::Array { len: None, .. } => {
                            format!("{}.length", entry.name)
                        }
                        _ => {
                            // checkrs: allow(clone_in_loops)
                            entry.name.clone()
                        }
                    };
                    changes.push(StorageChangeInfo {
                        name,
                        ty: &entry.ty,
                        offset: entry.offset,
                        bytes: entry.bytes,
                    });
                }
            }
        }
        changes
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
                        Ok(DynSolValue::Tuple(values)) => format_abi_args(&values, &func.inputs),
                        Ok(other) => format_abi_args(&[other], &func.inputs),
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
                let msg = panic.kind().map(|k| k.as_str()).unwrap_or("unknown code");
                return format!("panic: {msg}");
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

/// Format a single decoded value using ABI parameter metadata so that structs
/// are rendered with their type name and field names.
pub(super) fn format_abi_value(value: &DynSolValue, param: &Param) -> String {
    match value {
        DynSolValue::Tuple(vals) => {
            if param.is_struct() {
                let name = param
                    .internal_type()
                    .and_then(|it| it.as_struct())
                    .map(|(_, n)| n.split('[').next().unwrap_or(n))
                    .unwrap_or("tuple");
                let inner: Vec<String> = vals
                    .iter()
                    .zip(param.components.iter())
                    .map(|(v, p)| format!("{}: {}", p.name(), format_abi_value(v, p)))
                    .collect();
                format!("{}({{ {inner} }})", name, inner = inner.join(", "))
            } else {
                let inner: Vec<String> = vals
                    .iter()
                    .zip(param.components.iter())
                    .map(|(v, p)| format_abi_value(v, p))
                    .collect();
                format!("({inner})", inner = inner.join(", "))
            }
        }
        DynSolValue::Array(vals) | DynSolValue::FixedArray(vals) => {
            let inner: Vec<String> = vals.iter().map(|v| format_abi_value(v, param)).collect();
            format!("[{inner}]", inner = inner.join(", "))
        }
        _ => format_value(value),
    }
}

/// Format a list of decoded values using ABI parameter metadata.
pub(super) fn format_abi_args(values: &[DynSolValue], params: &[Param]) -> String {
    if values.is_empty() {
        return String::new();
    }
    values
        .iter()
        .zip(params.iter())
        .map(|(v, p)| format_abi_value(v, p))
        .collect::<Vec<String>>()
        .join(", ")
}

use crate::foundry::LinkReferences;

type StorageLayoutResult = (HashMap<U256, Vec<StorageEntry>>, Vec<ArrayInfo>);

/// Parse state-variable names, types, and array metadata from an artifact's
/// `storageLayout` output.
fn parse_storage_layout(artifact: &Artifact) -> Option<StorageLayoutResult> {
    let layout = artifact.storage_layout()?;
    let mut names: HashMap<U256, Vec<StorageEntry>> = HashMap::new();
    let mut arrays = Vec::new();
    for entry in &layout.storage {
        let slot = entry.slot.parse::<U256>().ok()?;
        let ty = super::StorageType::parse(&entry.type_name)?;
        let offset = entry.offset as usize;
        let bytes = layout
            .types
            .get(&entry.type_name)
            .and_then(|t| t.number_of_bytes.parse::<usize>().ok())
            .unwrap_or(32);
        names.entry(slot).or_default().push(StorageEntry {
            offset,
            // checkrs: allow(clone_in_loops)
            name: entry.label.clone(),
            // checkrs: allow(clone_in_loops)
            ty: ty.clone(),
            bytes,
        });

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
            let struct_fields = parse_struct_fields(&layout.types, &entry.type_name);
            arrays.push(ArrayInfo {
                // checkrs: allow(clone_in_loops)
                name: entry.label.clone(),
                // checkrs: allow(clone_in_loops)
                element_type: *element.clone(),
                element_slots,
                start_slot,
                len: *array_len,
                struct_fields,
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

/// Parse struct field layout for an array whose element type is a struct.
///
/// Returns `None` if the element type is not a struct or if member info is
/// unavailable.
fn parse_struct_fields(
    types: &HashMap<String, crate::foundry::StorageTypeInfo>,
    array_type_name: &str,
) -> Option<Vec<StructField>> {
    let info = types.get(array_type_name)?;
    let base_type_name = info.base.as_ref()?;
    let base_type = types.get(base_type_name)?;
    if base_type.members.is_empty() {
        return None;
    }
    let mut fields = Vec::new();
    for member in &base_type.members {
        let slot_offset = member.slot.parse::<usize>().ok()?;
        let ty = super::StorageType::parse(&member.type_name)?;
        fields.push(StructField {
            // checkrs: allow(clone_in_loops)
            name: member.label.clone(),
            slot_offset,
            ty,
        });
    }
    Some(fields)
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
