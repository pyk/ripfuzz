//! Trace context for formatting and decoding raw execution traces.
//!
//! [`TraceContext`] collects ABIs and address labels from build artifacts,
//! then provides lookup methods used by [`Trace::display_with`](crate::evm::trace::Trace::display_with).

use std::collections::HashMap;

use alloy_dyn_abi::{DynSolEvent, DynSolType, DynSolValue};
use alloy_json_abi::{EventParam, InternalType, JsonAbi, Param};
use alloy_primitives::{B256, FixedBytes, U256, b256, keccak256};
use alloy_sol_types::SolError;
use anyhow::Result;
use revm::primitives::{Address, Bytes};

use crate::evm::chain::DEFAULT_DEPLOYER;
use crate::evm::cheatcode::VM_ADDRESS;
use crate::evm::trace::{MappingSlots, StorageType};
use crate::foundry::{Artifact, ArtifactId, Project, StorageTypeInfo};

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
    ty: StorageType,
    bytes: usize,
}

/// Metadata for an array variable so that hashed element slots can be
/// resolved back to `array[index]`.
#[derive(Debug, Clone)]
struct ArrayInfo {
    name: String,
    element_type: StorageType,
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
    ty: StorageType,
    bytes: usize,
}

/// A single changed variable resolved from a packed storage slot.
#[derive(Debug)]
pub struct StorageChangeInfo<'a> {
    pub name: String,
    pub ty: &'a StorageType,
    pub offset: usize,
    pub bytes: usize,
}

/// Metadata for a mapping variable so that hashed mapping slots can be
/// resolved back to human-readable `mapping[key]` labels.
#[derive(Debug, Clone)]
struct MappingInfo {
    name: String,
    base_slot: U256,
    key_types: Vec<String>,
    value_storage_type: StorageType,
    value_struct_fields: Option<Vec<StructField>>,
    value_element_slots: usize,
}

#[derive(Debug, Clone)]
pub struct TraceContext {
    labels: HashMap<Address, String>,
    abis: Vec<JsonAbi>,
    bytecode_entries: Vec<BytecodeEntry>,
    initcode_entries: Vec<BytecodeEntry>,
    /// Maps contract name -> (slot -> list of packed variables).
    storage_names: HashMap<String, HashMap<U256, Vec<StorageEntry>>>,
    /// Maps contract name -> list of arrays for element-slot resolution.
    array_info: HashMap<String, Vec<ArrayInfo>>,
    /// Maps contract name -> list of mappings for slot resolution.
    mapping_info: HashMap<String, Vec<MappingInfo>>,
}

impl Default for TraceContext {
    fn default() -> Self {
        let mut labels = HashMap::new();
        labels.insert(VM_ADDRESS, "RaptorVM".into());
        labels.insert(DEFAULT_DEPLOYER, "RaptorDeployer".into());
        Self {
            labels,
            abis: Vec::new(),
            bytecode_entries: Vec::new(),
            initcode_entries: Vec::new(),
            storage_names: HashMap::new(),
            array_info: HashMap::new(),
            mapping_info: HashMap::new(),
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
        let mut initcode_entries = Vec::new();
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
            let initcode = artifact.bytecode();
            let code =
                initcode.map(|b| parse_bytecode_with_placeholders(&b.object, &b.link_references));
            if let Some(code) = code
                && !code.is_empty()
            {
                let positions = initcode
                    .map(|b| collect_link_positions(&b.link_references))
                    .unwrap_or_default();
                let mut masked = code;
                zero_out_positions(&mut masked, &positions);
                initcode_entries.push(BytecodeEntry {
                    name: artifact.name().into(),
                    base_hash: keccak256(&masked),
                    positions,
                });
            }
            if let Some((names, arrays, mappings)) = parse_storage_layout(&artifact) {
                ctx.storage_names.insert(artifact.name().into(), names);
                ctx.array_info.insert(artifact.name().into(), arrays);
                ctx.mapping_info.insert(artifact.name().into(), mappings);
            }
            ctx.abis.push(artifact.into_abi());
        }
        ctx.bytecode_entries = bytecode_entries;
        ctx.initcode_entries = initcode_entries;
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

    /// Look up a contract name by its initcode.
    ///
    /// Matches against the artifact initcode index, masking out library
    /// link-reference positions so that linked and unlinked bytecodes match.
    pub fn resolve_by_initcode(&self, code: &Bytes) -> Option<&str> {
        let mut masked = code.to_vec();
        for entry in &self.initcode_entries {
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
    pub fn resolve_storage_name(
        &self,
        contract_name: &str,
        slot: &U256,
        mapping_slots: Option<&MappingSlots>,
    ) -> Option<String> {
        if let Some(entries) = self
            .storage_names
            .get(contract_name)
            .and_then(|map| map.get(slot))
            && let Some(entry) = entries.first()
        {
            return match entry.ty {
                // Fixed array: base slot is the first element.
                StorageType::Array { len: Some(_), .. } => Some(format!("{}[0]", entry.name)),
                // Dynamic array: base slot holds the length.
                StorageType::Array { len: None, .. } => Some(format!("{}.length", entry.name)),
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
                        None => {
                            // Guard against mapping slots being misidentified as
                            // dynamic array elements. In practice, dynamic arrays
                            // rarely exceed 10 million elements.
                            let idx = u64::try_from(index).unwrap_or(u64::MAX);
                            idx < 10_000_000
                        }
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
        // Check if the slot belongs to a mapping (or a struct/array inside a mapping).
        if let Some(mapping_slots) = mapping_slots {
            let slot_b256 = B256::from(*slot);

            // Direct mapping slot.
            if let Some(keys) = mapping_slots.key_chain(slot_b256) {
                let base_slot = mapping_slots.base_slot(slot_b256)?;
                let base_u256 = U256::from_be_bytes(base_slot.0);
                for info in self.mapping_info.get(contract_name)? {
                    if info.base_slot == base_u256 {
                        // checkrs: allow(clone_in_loops)
                        let mut label = info.name.clone();
                        for (key, key_type) in keys.iter().zip(info.key_types.iter()) {
                            let key_ty = StorageType::parse(key_type)?;
                            let key_u256 = U256::from_be_bytes(key.0);
                            let key_str = key_ty.format_value(key_u256, 0, 32);
                            label = format!("{}[{}]", label, key_str);
                        }
                        // Dynamic array base slot inside mapping: show length.
                        if let StorageType::Array { len: None, .. } = &info.value_storage_type {
                            return Some(format!("{}.length", label));
                        }
                        // Struct base slot inside mapping: show first field.
                        if let Some(fields) = &info.value_struct_fields
                            && let Some(field) = fields.iter().find(|f| f.slot_offset == 0)
                        {
                            return Some(format!("{}.{}", label, field.name));
                        }
                        return Some(label);
                    }
                }
            }

            // Mapping slot + offset (struct field or fixed array element).
            for m_slot in mapping_slots.keys.keys() {
                let m_u256 = U256::from_be_bytes(m_slot.0);
                if *slot >= m_u256 {
                    let offset = slot - m_u256;
                    let Ok(offset_usize) = usize::try_from(offset) else {
                        continue;
                    };
                    let Some(keys) = mapping_slots.key_chain(*m_slot) else {
                        continue;
                    };
                    let Some(base_slot) = mapping_slots.base_slot(*m_slot) else {
                        continue;
                    };
                    let base_u256 = U256::from_be_bytes(base_slot.0);
                    for info in self.mapping_info.get(contract_name)? {
                        if info.base_slot != base_u256 {
                            continue;
                        }
                        // checkrs: allow(clone_in_loops)
                        let mut label = info.name.clone();
                        for (key, key_type) in keys.iter().zip(info.key_types.iter()) {
                            let key_ty = StorageType::parse(key_type)?;
                            let key_u256 = U256::from_be_bytes(key.0);
                            let key_str = key_ty.format_value(key_u256, 0, 32);
                            label = format!("{}[{}]", label, key_str);
                        }
                        // Struct field inside mapping value (including array fields).
                        if let Some(fields) = &info.value_struct_fields {
                            if let Some(field) =
                                fields.iter().find(|f| f.slot_offset == offset_usize)
                            {
                                if let StorageType::Array { len: Some(_), .. } = &field.ty {
                                    return Some(format!("{}.{}[0]", label, field.name));
                                }
                                return Some(format!("{}.{}", label, field.name));
                            }
                            // Check if offset falls within a struct field that is a fixed array.
                            for field in fields.iter() {
                                if let StorageType::Array {
                                    len: Some(len),
                                    element,
                                } = &field.ty
                                {
                                    let total_slots = field.bytes.div_ceil(32);
                                    if offset_usize >= field.slot_offset
                                        && offset_usize < field.slot_offset + total_slots
                                    {
                                        let slot_index = offset_usize - field.slot_offset;
                                        let element_slots = element.slot_size().div_ceil(32);
                                        let index = if element_slots > 1 {
                                            slot_index / element_slots
                                        } else {
                                            slot_index * (32 / element.slot_size())
                                        };
                                        if index < *len {
                                            return Some(format!(
                                                "{}.{}[{}]",
                                                label, field.name, index
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        // Fixed array element inside mapping value.
                        if let StorageType::Array { len: Some(len), .. } = &info.value_storage_type
                        {
                            let index = offset_usize / info.value_element_slots;
                            if index < *len {
                                let field_offset = offset_usize % info.value_element_slots;
                                if let Some(fields) = &info.value_struct_fields
                                    && let Some(field) =
                                        fields.iter().find(|f| f.slot_offset == field_offset)
                                {
                                    return Some(format!("{}[{}].{}", label, index, field.name));
                                }
                                return Some(format!("{}[{}]", label, index));
                            }
                        }
                        // Dynamic array length inside mapping value.
                        if let StorageType::Array { len: None, .. } = &info.value_storage_type
                            && offset_usize == 0
                        {
                            return Some(format!("{}.length", label));
                        }
                        break;
                    }
                }
            }

            // Dynamic array element of a mapping slot.
            for m_slot in mapping_slots.keys.keys() {
                let _m_u256 = U256::from_be_bytes(m_slot.0);
                let data_start = U256::from_be_bytes(keccak256(m_slot.0).0);
                if *slot >= data_start {
                    let offset = slot - data_start;
                    let Ok(index) = u64::try_from(offset) else {
                        continue;
                    };
                    if index >= 10_000_000 {
                        continue;
                    }
                    let Some(keys) = mapping_slots.key_chain(*m_slot) else {
                        continue;
                    };
                    let Some(base_slot) = mapping_slots.base_slot(*m_slot) else {
                        continue;
                    };
                    let base_u256 = U256::from_be_bytes(base_slot.0);
                    for info in self.mapping_info.get(contract_name)? {
                        if info.base_slot != base_u256 {
                            continue;
                        }
                        if let StorageType::Array { len: None, .. } = &info.value_storage_type {
                            // checkrs: allow(clone_in_loops)
                            let mut label = info.name.clone();
                            for (key, key_type) in keys.iter().zip(info.key_types.iter()) {
                                let key_ty = StorageType::parse(key_type)?;
                                let key_u256 = U256::from_be_bytes(key.0);
                                let key_str = key_ty.format_value(key_u256, 0, 32);
                                label = format!("{}[{}]", label, key_str);
                            }
                            let field_offset = (index as usize) % info.value_element_slots;
                            if let Some(fields) = &info.value_struct_fields
                                && let Some(field) =
                                    fields.iter().find(|f| f.slot_offset == field_offset)
                            {
                                return Some(format!("{}[{}].{}", label, index, field.name));
                            }
                            return Some(format!("{}[{}]", label, index));
                        }
                        break;
                    }
                }
            }

            // Dynamic array element nested inside a struct field of a
            // top-level array (e.g. `offers[0].offer.market.collateralParams[0]`).
            // These data areas are computed via keccak256(length_slot) with
            // 32-byte input and recorded in array_starts.
            for (data_start, parent_slot) in mapping_slots.array_start_entries() {
                // Skip data starts whose parent is a mapping-slot result
                // (those are already handled by the mapping dynamic-array
                // paths above).
                if mapping_slots.is_mapping_result(parent_slot) {
                    continue;
                }
                let data_start_u256 = U256::from_be_bytes(data_start.0);
                if *slot < data_start_u256 {
                    continue;
                }
                let offset = slot - data_start_u256;
                let Ok(offset_usize) = usize::try_from(offset) else {
                    continue;
                };
                let parent_u256 = U256::from_be_bytes(parent_slot.0);
                // Find which array element the parent slot belongs to.
                let Some(arrays) = self.array_info.get(contract_name) else {
                    continue;
                };
                let mut array_match: Option<(&ArrayInfo, U256)> = None;
                for array in arrays {
                    if parent_u256 >= array.start_slot {
                        let rel = parent_u256 - array.start_slot;
                        let idx = rel / U256::from(array.element_slots);
                        let in_bounds = match array.len {
                            Some(len) => match usize::try_from(idx) {
                                Ok(i) => i < len,
                                Err(_) => false,
                            },
                            None => {
                                let i = u64::try_from(idx).unwrap_or(u64::MAX);
                                i < 10_000_000
                            }
                        };
                        if !in_bounds {
                            continue;
                        }
                        if array_match
                            .as_ref()
                            .map(|(a, _)| array.start_slot > a.start_slot)
                            .unwrap_or(true)
                        {
                            array_match = Some((array, idx));
                        }
                    }
                }
                let Some((array, elem_idx)) = array_match else {
                    continue;
                };
                let field_offset_rel =
                    (parent_u256 - array.start_slot) % U256::from(array.element_slots);
                let field_off = u64::try_from(field_offset_rel).unwrap_or(0) as usize;
                let Some(fields) = &array.struct_fields else {
                    continue;
                };
                let Some(dyn_array_field) = fields.iter().find(|f| f.slot_offset == field_off)
                else {
                    continue;
                };
                if !matches!(dyn_array_field.ty, StorageType::Array { len: None, .. }) {
                    continue;
                }
                let StorageType::Array { element, len: None } = &dyn_array_field.ty else {
                    continue;
                };
                let element_slots = element.slot_size().div_ceil(32);
                let index = offset_usize / element_slots;
                if index >= 10_000_000 {
                    continue;
                }
                let name = format!("{}[{}]", array.name, elem_idx);
                let label = format!("{}.{}", name, dyn_array_field.name);
                return Some(format!("{}[{}]", label, index));
            }
        }
        None
    }

    /// Look up the storage type for a storage slot in a contract.
    pub fn resolve_storage_type(
        &self,
        contract_name: &str,
        slot: &U256,
        mapping_slots: Option<&MappingSlots>,
    ) -> Option<&StorageType> {
        if let Some(entries) = self
            .storage_names
            .get(contract_name)
            .and_then(|map| map.get(slot))
            && let Some(entry) = entries.first()
        {
            return match &entry.ty {
                // Fixed array: base slot is the first element.
                StorageType::Array {
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
                        None => {
                            // Guard against mapping slots being misidentified as
                            // dynamic array elements. In practice, dynamic arrays
                            // rarely exceed 10 million elements.
                            let idx = u64::try_from(index).unwrap_or(u64::MAX);
                            idx < 10_000_000
                        }
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
        // Check if the slot belongs to a mapping (or a struct/array inside a mapping).
        if let Some(mapping_slots) = mapping_slots {
            let slot_b256 = B256::from(*slot);

            // Direct mapping slot.
            if let Some(_keys) = mapping_slots.key_chain(slot_b256) {
                let base_slot = mapping_slots.base_slot(slot_b256)?;
                let base_u256 = U256::from_be_bytes(base_slot.0);
                for info in self.mapping_info.get(contract_name)? {
                    if info.base_slot == base_u256 {
                        return Some(&info.value_storage_type);
                    }
                }
            }

            // Mapping slot + offset (struct field or fixed array element).
            for m_slot in mapping_slots.keys.keys() {
                let m_u256 = U256::from_be_bytes(m_slot.0);
                if *slot >= m_u256 {
                    let offset = slot - m_u256;
                    let Ok(offset_usize) = usize::try_from(offset) else {
                        continue;
                    };
                    let Some(base_slot) = mapping_slots.base_slot(*m_slot) else {
                        continue;
                    };
                    let base_u256 = U256::from_be_bytes(base_slot.0);
                    for info in self.mapping_info.get(contract_name)? {
                        if info.base_slot != base_u256 {
                            continue;
                        }
                        // Struct field inside mapping value (including array fields).
                        if let Some(fields) = &info.value_struct_fields {
                            if let Some(field) =
                                fields.iter().find(|f| f.slot_offset == offset_usize)
                            {
                                if let StorageType::Array { element, .. } = &field.ty {
                                    return Some(element.as_ref());
                                }
                                return Some(&field.ty);
                            }
                            // Check if offset falls within a struct field that is a fixed array.
                            for field in fields.iter() {
                                if let StorageType::Array { element, .. } = &field.ty {
                                    let total_slots = field.bytes.div_ceil(32);
                                    if offset_usize >= field.slot_offset
                                        && offset_usize < field.slot_offset + total_slots
                                    {
                                        return Some(element.as_ref());
                                    }
                                }
                            }
                        }
                        // Fixed array element inside mapping value.
                        if let StorageType::Array {
                            element,
                            len: Some(_),
                        } = &info.value_storage_type
                        {
                            let field_offset = offset_usize % info.value_element_slots;
                            if let Some(fields) = &info.value_struct_fields
                                && let Some(field) =
                                    fields.iter().find(|f| f.slot_offset == field_offset)
                            {
                                return Some(&field.ty);
                            }
                            return Some(element.as_ref());
                        }
                        // Dynamic array length inside mapping value.
                        if let StorageType::Array { len: None, .. } = &info.value_storage_type
                            && offset_usize == 0
                        {
                            return Some(&info.value_storage_type);
                        }
                        break;
                    }
                }
            }

            // Dynamic array element of a mapping slot.
            for m_slot in mapping_slots.keys.keys() {
                let _m_u256 = U256::from_be_bytes(m_slot.0);
                let data_start = U256::from_be_bytes(keccak256(m_slot.0).0);
                if *slot >= data_start {
                    let offset = slot - data_start;
                    let Ok(index) = u64::try_from(offset) else {
                        continue;
                    };
                    if index >= 10_000_000 {
                        continue;
                    }
                    let Some(base_slot) = mapping_slots.base_slot(*m_slot) else {
                        continue;
                    };
                    let base_u256 = U256::from_be_bytes(base_slot.0);
                    for info in self.mapping_info.get(contract_name)? {
                        if info.base_slot != base_u256 {
                            continue;
                        }
                        if let StorageType::Array {
                            element, len: None, ..
                        } = &info.value_storage_type
                        {
                            let field_offset = (index as usize) % info.value_element_slots;
                            if let Some(fields) = &info.value_struct_fields
                                && let Some(field) =
                                    fields.iter().find(|f| f.slot_offset == field_offset)
                            {
                                return Some(&field.ty);
                            }
                            return Some(element.as_ref());
                        }
                        break;
                    }
                }
            }

            // Dynamic array element nested inside a struct field of a
            // top-level array.
            for (data_start, parent_slot) in mapping_slots.array_start_entries() {
                // Skip data starts whose parent is a mapping slot.
                if mapping_slots.is_mapping_result(parent_slot) {
                    continue;
                }
                let data_start_u256 = U256::from_be_bytes(data_start.0);
                if *slot < data_start_u256 {
                    continue;
                }
                let offset = slot - data_start_u256;
                let Ok(offset_usize) = usize::try_from(offset) else {
                    continue;
                };
                let parent_u256 = U256::from_be_bytes(parent_slot.0);
                let Some(arrays) = self.array_info.get(contract_name) else {
                    continue;
                };
                let mut array_match: Option<(&ArrayInfo, U256)> = None;
                for array in arrays {
                    if parent_u256 >= array.start_slot {
                        let rel = parent_u256 - array.start_slot;
                        let idx = rel / U256::from(array.element_slots);
                        let in_bounds = match array.len {
                            Some(len) => match usize::try_from(idx) {
                                Ok(i) => i < len,
                                Err(_) => false,
                            },
                            None => {
                                let i = u64::try_from(idx).unwrap_or(u64::MAX);
                                i < 10_000_000
                            }
                        };
                        if !in_bounds {
                            continue;
                        }
                        if array_match
                            .as_ref()
                            .map(|(a, _)| array.start_slot > a.start_slot)
                            .unwrap_or(true)
                        {
                            array_match = Some((array, idx));
                        }
                    }
                }
                let Some((array, _elem_idx)) = array_match else {
                    continue;
                };
                let field_offset_rel =
                    (parent_u256 - array.start_slot) % U256::from(array.element_slots);
                let field_off = u64::try_from(field_offset_rel).unwrap_or(0) as usize;
                let Some(fields) = &array.struct_fields else {
                    continue;
                };
                let Some(dyn_array_field) = fields.iter().find(|f| f.slot_offset == field_off)
                else {
                    continue;
                };
                if !matches!(dyn_array_field.ty, StorageType::Array { len: None, .. }) {
                    continue;
                }
                let StorageType::Array { element, len: None } = &dyn_array_field.ty else {
                    continue;
                };
                let element_slots = element.slot_size().div_ceil(32);
                let index = offset_usize / element_slots;
                if index >= 10_000_000 {
                    continue;
                }
                return Some(element.as_ref());
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
        _mapping_slots: Option<&MappingSlots>,
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
                        StorageType::Array { len: Some(_), .. } => {
                            format!("{}[0]", entry.name)
                        }
                        StorageType::Array { len: None, .. } => {
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
                        Ok(DynSolValue::Tuple(values)) => {
                            format_abi_args(&values, &func.inputs, &self.labels)
                        }
                        Ok(other) => format_abi_args(&[other], &func.inputs, &self.labels),
                        Err(_) => "...".into(),
                    }
                };
                return (Some(func.name.as_str()), args);
            }
        }
        (None, "...".into())
    }

    /// Decode a function return value using the registered ABIs.
    ///
    /// Returns a formatted string of decoded return values if the function
    /// selector is found and the return data can be decoded.
    pub fn decode_return(&self, input: &Bytes, output: &Bytes) -> Option<String> {
        if input.len() < 4 || output.is_empty() {
            return None;
        }
        let sel: [u8; 4] = input[..4].try_into().unwrap_or_default();
        for abi in &self.abis {
            if let Some(func) = abi.function_by_selector(FixedBytes::new(sel)) {
                if func.outputs.is_empty() {
                    return None;
                }
                let types: Vec<DynSolType> = func
                    .outputs
                    .iter()
                    .filter_map(|p| DynSolType::parse(&p.selector_type()).ok())
                    .collect();
                if types.is_empty() {
                    return None;
                }
                let tuple = DynSolType::Tuple(types);
                match tuple.abi_decode_params(output) {
                    Ok(DynSolValue::Tuple(values)) => {
                        let formatted = format_abi_args(&values, &func.outputs, &self.labels);
                        return Some(formatted);
                    }
                    Ok(other) => {
                        let formatted = format_abi_args(&[other], &func.outputs, &self.labels);
                        return Some(formatted);
                    }
                    Err(_) => return Some("...".into()),
                }
            }
        }
        None
    }

    /// Decode an event log using the registered ABIs.
    ///
    /// Returns the event name (if found) and a formatted argument string.
    pub fn decode_event(&self, log: &revm::primitives::Log) -> (Option<String>, String) {
        let topics = log.data.topics();
        let data = &log.data.data;

        if topics.is_empty() {
            return (None, format!("0x{}", hex::encode(data)));
        }

        let topic_0 = topics[0];
        for abi in &self.abis {
            for event in abi.events() {
                if event.selector() == topic_0 {
                    // checkrs: allow(clone_in_loops)
                    let name = event.name.clone();
                    let indexed: Vec<DynSolType> = event
                        .inputs
                        .iter()
                        .filter(|p| p.indexed)
                        .filter_map(|p| DynSolType::parse(&p.selector_type()).ok())
                        .collect();
                    let body: Vec<DynSolType> = event
                        .inputs
                        .iter()
                        .filter(|p| !p.indexed)
                        .filter_map(|p| DynSolType::parse(&p.selector_type()).ok())
                        .collect();
                    let body = DynSolType::Tuple(body);
                    let Some(dyn_event) = DynSolEvent::new(Some(topic_0), indexed, body) else {
                        return (Some(name), "...".into());
                    };
                    match dyn_event.decode_log_data(&log.data) {
                        Ok(decoded) => {
                            let mut args = Vec::new();
                            let mut indexed_idx = 0;
                            let mut body_idx = 0;
                            for param in &event.inputs {
                                if param.indexed {
                                    if let Some(val) = decoded.indexed.get(indexed_idx) {
                                        args.push(format!(
                                            "{}: {}",
                                            param.name(),
                                            format_abi_value(val, param, &self.labels)
                                        ));
                                        indexed_idx += 1;
                                    }
                                } else {
                                    if let Some(val) = decoded.body.get(body_idx) {
                                        args.push(format!(
                                            "{}: {}",
                                            param.name(),
                                            format_abi_value(val, param, &self.labels)
                                        ));
                                        body_idx += 1;
                                    }
                                }
                            }
                            return (Some(name), args.join(", "));
                        }
                        Err(_) => {
                            return (Some(name), "...".into());
                        }
                    }
                }
            }
        }
        (None, format!("0x{}", hex::encode(data)))
    }

    /// Decode a `Log(string, ...)` event into a simple log message.
    ///
    /// Returns `None` if the log is not a recognized `Log` event.
    pub fn decode_log_event(&self, log: &revm::primitives::Log) -> Option<String> {
        let topics = log.data.topics();
        if topics.is_empty() {
            return None;
        }
        let topic0 = topics[0];

        const LOG_STRING: B256 =
            b256!("0xcf34ef537ac33ee1ac626ca1587a0a7e8e51561e5514f8cb36afa1c5102b3bab");
        const LOG_STRING_STRING: B256 =
            b256!("0x821f337ab34a905a52ea2a22aa6b9ef872196a034e37c2bc08d88e21d8cece09");
        const LOG_STRING_UINT256: B256 =
            b256!("0xdd970dd9b5bfe707922155b058a407655cb18288b807e2216442bca8ad83d6b5");
        const LOG_STRING_ADDRESS: B256 =
            b256!("0x1dfffa052d4a63bd70f14b863e128979d1c59e3589a0a3beb2633a120047042d");
        const LOG_STRING_BYTES: B256 =
            b256!("0x381e6feca73f11fec6ca464f9d2a06ae1bb0ee33a55355e128d2d3aca53cc5f4");
        const LOG_STRING_BOOL: B256 =
            b256!("0x52dd9d08c343f72c69027ade2a075f6242dba2eeca3a3c61bfd8d00d32f6bd20");

        let data = log.data.data.as_ref();

        if topic0 == LOG_STRING {
            let Ok(DynSolValue::String(msg)) = DynSolType::String.abi_decode(data) else {
                return None;
            };
            return Some(msg);
        }

        let ty = match topic0 {
            t if t == LOG_STRING_STRING => {
                DynSolType::Tuple(vec![DynSolType::String, DynSolType::String])
            }
            t if t == LOG_STRING_UINT256 => {
                DynSolType::Tuple(vec![DynSolType::String, DynSolType::Uint(256)])
            }
            t if t == LOG_STRING_ADDRESS => {
                DynSolType::Tuple(vec![DynSolType::String, DynSolType::Address])
            }
            t if t == LOG_STRING_BYTES => {
                DynSolType::Tuple(vec![DynSolType::String, DynSolType::Bytes])
            }
            t if t == LOG_STRING_BOOL => {
                DynSolType::Tuple(vec![DynSolType::String, DynSolType::Bool])
            }
            _ => return None,
        };

        let Ok(DynSolValue::Tuple(values)) = ty.abi_decode_sequence(data) else {
            return None;
        };
        if values.len() != 2 {
            return None;
        }
        let msg = match &values[0] {
            DynSolValue::String(s) => s.clone(),
            _ => return None,
        };
        let val = format_value(&values[1], &self.labels);
        Some(format!("{msg}{val}"))
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
                let msg = match panic.kind() {
                    Some(kind) => match kind {
                        alloy_sol_types::PanicKind::Generic => "generic panic",
                        alloy_sol_types::PanicKind::Assert => "assertion failed",
                        alloy_sol_types::PanicKind::UnderOverflow => {
                            "arithmetic overflow/underflow"
                        }
                        alloy_sol_types::PanicKind::DivisionByZero => "division by zero",
                        alloy_sol_types::PanicKind::EnumConversionError => "enum conversion error",
                        alloy_sol_types::PanicKind::StorageEncodingError => {
                            "storage encoding error"
                        }
                        alloy_sol_types::PanicKind::EmptyArrayPop => "empty array pop",
                        alloy_sol_types::PanicKind::ArrayOutOfBounds => "array out of bounds",
                        alloy_sol_types::PanicKind::ResourceError => "resource error",
                        alloy_sol_types::PanicKind::InvalidInternalFunction => {
                            "invalid internal function"
                        }
                        _ => kind.as_str(),
                    },
                    None => "unknown code",
                };
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
                        }
                        let types: Vec<DynSolType> = error
                            .inputs
                            .iter()
                            .filter_map(|p| DynSolType::parse(&p.selector_type()).ok())
                            .collect();
                        if types.is_empty() {
                            return format!("{}(...)", error.name);
                        }
                        let tuple = DynSolType::Tuple(types);
                        match tuple.abi_decode_params(&data[4..]) {
                            Ok(DynSolValue::Tuple(values)) => {
                                return format!(
                                    "{}({})",
                                    error.name,
                                    format_abi_args(&values, &error.inputs, &self.labels)
                                );
                            }
                            Ok(other) => {
                                return format!(
                                    "{}({})",
                                    error.name,
                                    format_abi_args(&[other], &error.inputs, &self.labels)
                                );
                            }
                            Err(_) => {
                                return format!("{}(...)", error.name);
                            }
                        }
                    }
                }
            }
        }

        format!("0x{}", hex::encode(data))
    }
}

pub(super) fn format_value(v: &DynSolValue, labels: &HashMap<Address, String>) -> String {
    match v {
        DynSolValue::Bool(b) => format!("{b}"),
        DynSolValue::Uint(n, _) => format!("{n}"),
        DynSolValue::Int(n, _) => format!("{n}"),
        DynSolValue::Address(a) => {
            if let Some(label) = labels.get(a) {
                format!("{label}: [{a}]")
            } else {
                format!("{a}")
            }
        }
        DynSolValue::String(s) => format!("\"{s}\""),
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        DynSolValue::Array(arr) | DynSolValue::FixedArray(arr) => {
            let inner: Vec<String> = arr.iter().map(|v| format_value(v, labels)).collect();
            format!("[{inner}]", inner = inner.join(", "))
        }
        DynSolValue::Tuple(vals) => {
            let inner: Vec<String> = vals.iter().map(|v| format_value(v, labels)).collect();
            format!("({inner})", inner = inner.join(", "))
        }
        _ => format!("{v:?}"),
    }
}

/// Trait abstracting over ABI parameter types so that [`format_abi_value`]
/// can be reused for both function parameters and event parameters.
pub(super) trait FormatParam {
    fn name(&self) -> &str;
    fn is_struct(&self) -> bool;
    fn internal_type(&self) -> Option<&InternalType>;
    fn components(&self) -> &[Param];
}

impl FormatParam for Param {
    fn name(&self) -> &str {
        Param::name(self)
    }
    fn is_struct(&self) -> bool {
        Param::is_struct(self)
    }
    fn internal_type(&self) -> Option<&InternalType> {
        Param::internal_type(self)
    }
    fn components(&self) -> &[Param] {
        &self.components
    }
}

impl FormatParam for EventParam {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_struct(&self) -> bool {
        EventParam::is_struct(self)
    }
    fn internal_type(&self) -> Option<&InternalType> {
        EventParam::internal_type(self)
    }
    fn components(&self) -> &[Param] {
        &self.components
    }
}

/// Format a single decoded value using ABI parameter metadata so that structs
/// are rendered with their type name and field names.
pub(super) fn format_abi_value(
    value: &DynSolValue,
    param: &impl FormatParam,
    labels: &HashMap<Address, String>,
) -> String {
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
                    .zip(param.components().iter())
                    .map(|(v, p)| format!("{}: {}", p.name(), format_abi_value(v, p, labels)))
                    .collect();
                format!("{}({{ {inner} }})", name, inner = inner.join(", "))
            } else {
                let inner: Vec<String> = vals
                    .iter()
                    .zip(param.components().iter())
                    .map(|(v, p)| format_abi_value(v, p, labels))
                    .collect();
                format!("({inner})", inner = inner.join(", "))
            }
        }
        DynSolValue::Array(vals) | DynSolValue::FixedArray(vals) => {
            let inner: Vec<String> = vals
                .iter()
                .map(|v| format_abi_value(v, param, labels))
                .collect();
            format!("[{inner}]", inner = inner.join(", "))
        }
        _ => format_value(value, labels),
    }
}

/// Format a list of decoded values using ABI parameter metadata.
pub(super) fn format_abi_args(
    values: &[DynSolValue],
    params: &[impl FormatParam],
    labels: &HashMap<Address, String>,
) -> String {
    if values.is_empty() {
        return String::new();
    }
    values
        .iter()
        .zip(params.iter())
        .map(|(v, p)| format_abi_value(v, p, labels))
        .collect::<Vec<String>>()
        .join(", ")
}

use crate::foundry::LinkReferences;

type StorageLayoutResult = (
    HashMap<U256, Vec<StorageEntry>>,
    Vec<ArrayInfo>,
    Vec<MappingInfo>,
);

/// Parse state-variable names, types, array and mapping metadata from an
/// artifact's `storageLayout` output.
fn parse_storage_layout(artifact: &Artifact) -> Option<StorageLayoutResult> {
    let layout = artifact.storage_layout()?;
    let mut names: HashMap<U256, Vec<StorageEntry>> = HashMap::new();
    let mut arrays = Vec::new();
    let mut mappings = Vec::new();
    for entry in &layout.storage {
        let slot = entry.slot.parse::<U256>().ok()?;
        let ty = StorageType::parse(&entry.type_name)?;
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
        if let StorageType::Array {
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

        // Build mapping metadata for hashed-slot resolution.
        if let StorageType::Mapping = &ty
            && let Some(type_info) = layout.types.get(&entry.type_name)
            && type_info.encoding == "mapping"
        {
            let key_types = resolve_mapping_key_types(&layout.types, &entry.type_name);
            let value_type = resolve_mapping_value_type(&layout.types, &entry.type_name);
            let value_storage_type =
                StorageType::parse(&value_type).unwrap_or(StorageType::Mapping);
            let value_struct_fields = parse_struct_fields(&layout.types, &value_type);
            let value_element_slots = element_byte_slots(&layout.types, &value_type);
            mappings.push(MappingInfo {
                // checkrs: allow(clone_in_loops)
                name: entry.label.clone(),
                base_slot: slot,
                key_types,
                value_storage_type,
                value_struct_fields,
                value_element_slots,
            });
        }
    }
    arrays.sort_by(|a, b| b.start_slot.cmp(&a.start_slot));
    Some((names, arrays, mappings)).filter(|(n, _, _)| !n.is_empty())
}

/// Resolve the key type references for a mapping type (including nested mappings).
///
/// Returns the raw `storageLayout` type strings (e.g. `t_address`) so they
/// can be parsed by [`StorageType::parse`].
fn resolve_mapping_key_types(
    types: &HashMap<String, StorageTypeInfo>,
    type_ref: &str,
) -> Vec<String> {
    let mut keys = Vec::new();
    let mut current = type_ref;
    while let Some(info) = types.get(current) {
        if info.encoding != "mapping" {
            break;
        }
        if let Some(key) = &info.key {
            // checkrs: allow(clone_in_loops)
            keys.push(key.clone());
        }
        if let Some(value) = &info.value {
            current = value;
        } else {
            break;
        }
    }
    keys
}

/// Resolve the final value type reference for a mapping type (including
/// nested mappings).
///
/// Returns the raw `storageLayout` type string (e.g. `t_uint256`) so it
/// can be parsed by [`StorageType::parse`] and looked up in the `types` map.
fn resolve_mapping_value_type(types: &HashMap<String, StorageTypeInfo>, type_ref: &str) -> String {
    let mut current = type_ref;
    while let Some(info) = types.get(current) {
        if info.encoding != "mapping" {
            return current.into();
        }
        if let Some(value) = &info.value {
            current = value;
        } else {
            return current.into();
        }
    }
    current.into()
}

/// Compute how many 32-byte slots a single array element occupies.
///
/// Looks up the array's `base` type in the `storageLayout` types map and
/// uses the base type's `numberOfBytes`.
fn element_byte_slots(types: &HashMap<String, StorageTypeInfo>, array_type_name: &str) -> usize {
    let info = types.get(array_type_name);
    let base_type = info.and_then(|t| t.base.as_ref());
    let bytes = base_type
        .and_then(|base| types.get(base))
        .and_then(|t| t.number_of_bytes.parse::<usize>().ok())
        .or_else(|| info.and_then(|t| t.number_of_bytes.parse::<usize>().ok()))
        .unwrap_or(32);
    bytes.div_ceil(32)
}

/// Parse struct field layout for a struct type, or for the base element type
/// of an array.
///
/// Nested struct members are recursively flattened so that every sub-field
/// appears in the returned list with a dot-separated name
/// (e.g. `data.a`) and an absolute `slot_offset` relative to the top-level
/// struct. This allows [`resolve_storage_name`] to match any field offset
/// without needing to recurse at lookup time.
///
/// Returns `None` if the type is not a struct or if member info is
/// unavailable.
fn parse_struct_fields(
    types: &HashMap<String, StorageTypeInfo>,
    type_name: &str,
) -> Option<Vec<StructField>> {
    fn collect_fields(
        types: &HashMap<String, StorageTypeInfo>,
        info: &StorageTypeInfo,
        prefix: &str,
        base_offset: usize,
    ) -> Option<Vec<StructField>> {
        if info.members.is_empty() {
            return Some(Vec::new());
        }
        let mut fields = Vec::new();
        for member in &info.members {
            let slot_offset = member.slot.parse::<usize>().ok()?;
            let ty = StorageType::parse(&member.type_name)?;
            let bytes = types
                .get(&member.type_name)
                .and_then(|t| t.number_of_bytes.parse::<usize>().ok())
                .unwrap_or(32);
            let abs_offset = base_offset + slot_offset;
            let name = if prefix.is_empty() {
                // checkrs: allow(clone_in_loops)
                member.label.clone()
            } else {
                format!("{}.{}", prefix, member.label)
            };
            // If the member is itself a struct, recursively flatten its
            // sub-fields so that offsets beyond the struct's base slot can
            // be matched to human-readable names.
            if ty == StorageType::Struct
                && let Some(nested_info) = types.get(&member.type_name)
                && let Some(sub_fields) = collect_fields(types, nested_info, &name, abs_offset)
            {
                fields.extend(sub_fields);
            } else {
                fields.push(StructField {
                    name,
                    slot_offset: abs_offset,
                    ty,
                    bytes,
                });
            }
        }
        Some(fields)
    }

    let info = types.get(type_name)?;
    // For array types, look up the base element type; for struct types use
    // the type itself.
    let struct_type = info
        .base
        .as_ref()
        .and_then(|base| types.get(base))
        .unwrap_or(info);
    if struct_type.members.is_empty() {
        return None;
    }
    collect_fields(types, struct_type, "", 0)
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
        assert_eq!(ctx.get_label(&VM_ADDRESS), Some("RaptorVM"));

        let ctx = TraceContext::from_project(&Project::new("fixtures/trace-context")).unwrap();
        assert_eq!(ctx.get_label(&VM_ADDRESS), Some("RaptorVM"));
    }
}
