//! Raw call trace types for the EVM chain.

use std::collections::HashMap;
use std::fmt;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{I256, U256};
use revm::interpreter::CallScheme;
use revm::primitives::{Address, Bytes, Log};

pub use context::{StorageChangeInfo, TraceContext};
pub use inspector::Inspector;

use crate::evm::cheatcode::VM_ADDRESS;

mod context;
mod inspector;

/// A single storage change recorded during a frame's execution.
#[derive(Debug, Clone)]
pub struct StorageChange {
    pub slot: U256,
    pub old_value: U256,
    pub new_value: U256,
}

/// Format a storage value according to its Solidity type.
/// A strongly typed Solidity storage type derived from the `storageLayout` output.
#[derive(Debug, Clone, PartialEq)]
pub enum StorageType {
    Bool,
    Uint(usize),
    Int(usize),
    Address,
    FixedBytes(usize),
    DynamicBytes,
    String,
    Array {
        element: Box<StorageType>,
        len: Option<usize>,
    },
    Mapping,
    Struct,
}

/// Find the index of the matching closing parenthesis for the first
/// opening parenthesis in `s`, accounting for nested pairs.
fn find_matching_paren(s: &str) -> Option<usize> {
    let mut depth = 1;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

impl StorageType {
    /// Parse a `storageLayout` type string into a strongly typed [`StorageType`].
    pub fn parse(type_name: &str) -> Option<Self> {
        let t = type_name;
        if t == "t_bool" {
            return Some(Self::Bool);
        }
        if t == "t_address" {
            return Some(Self::Address);
        }
        if t == "t_bytes_storage" {
            return Some(Self::DynamicBytes);
        }
        if t == "t_string_storage" {
            return Some(Self::String);
        }
        if let Some(bits) = t.strip_prefix("t_uint") {
            let bits = bits.parse::<usize>().ok()?;
            return Some(Self::Uint(bits));
        }
        if let Some(bits) = t.strip_prefix("t_int") {
            let bits = bits.parse::<usize>().ok()?;
            return Some(Self::Int(bits));
        }
        if let Some(sz) = t.strip_prefix("t_bytes") {
            let sz = sz.parse::<usize>().ok()?;
            return Some(Self::FixedBytes(sz));
        }
        if t.starts_with("t_array(") {
            let inner = t.strip_prefix("t_array(")?;
            let end = find_matching_paren(inner)?;
            let element = Self::parse(&inner[..end])?;
            let rest = &inner[end + 1..];
            let len = if rest.starts_with("dyn_") {
                None
            } else {
                let n = rest.split('_').next()?;
                Some(n.parse::<usize>().ok()?)
            };
            return Some(Self::Array {
                element: Box::new(element),
                len,
            });
        }
        if t.starts_with("t_mapping(") {
            return Some(Self::Mapping);
        }
        if t.starts_with("t_struct(") {
            return Some(Self::Struct);
        }
        if t.starts_with("t_contract(") {
            return Some(Self::Address);
        }
        None
    }

    /// Return the byte size of a single value of this type.
    pub fn slot_size(&self) -> usize {
        match self {
            Self::Bool => 1,
            Self::Uint(bits) => bits / 8,
            Self::Int(bits) => bits / 8,
            Self::Address => 20,
            Self::FixedBytes(sz) => *sz,
            Self::DynamicBytes
            | Self::String
            | Self::Array { .. }
            | Self::Mapping
            | Self::Struct => 32,
        }
    }

    /// Format a raw storage slot value according to this type.
    ///
    /// `offset` is the byte offset within the 32-byte word, and `bytes` is the
    /// number of bytes this variable occupies, so that packed storage slots are
    /// rendered correctly.
    pub fn format_value(&self, value: U256, offset: usize, bytes: usize) -> String {
        let extracted = if offset == 0 && bytes >= 32 {
            value
        } else {
            let shifted = value >> (offset * 8);
            let mask = (U256::from(1) << (bytes * 8)) - U256::from(1);
            shifted & mask
        };
        match self {
            Self::Bool => {
                if extracted.is_zero() {
                    "false".into()
                } else {
                    "true".into()
                }
            }
            Self::Address => {
                let bytes = extracted.to_be_bytes::<32>();
                let addr = Address::from_slice(&bytes[12..]);
                addr.to_checksum(None)
            }
            Self::FixedBytes(_) | Self::DynamicBytes => {
                let bytes = extracted.to_be_bytes::<32>();
                let hex_str = hex::encode(bytes);
                let trimmed = hex_str.trim_start_matches('0');
                if trimmed.is_empty() {
                    "0x00".into()
                } else {
                    format!("0x{trimmed}")
                }
            }
            Self::String => {
                let bytes = value.to_be_bytes::<32>();
                let low_byte = bytes[31];
                if low_byte & 1 == 0 {
                    // Short string: length = low_byte / 2, data in high 31 bytes
                    let len = (low_byte / 2) as usize;
                    if len == 0 {
                        return "\"\"".into();
                    }
                    let data = &bytes[..len];
                    if let Ok(s) = std::str::from_utf8(data) {
                        return format!("\"{s}\"");
                    }
                }
                // Long string or invalid UTF-8: fall back to hex
                let hex_str = hex::encode(bytes);
                let trimmed = hex_str.trim_start_matches('0');
                if trimmed.is_empty() {
                    "0x00".into()
                } else {
                    format!("0x{trimmed}")
                }
            }
            Self::Int(bits) => {
                let bits = *bits;
                if bits < 256 {
                    let mask = U256::from(1).wrapping_shl(bits);
                    let half = U256::from(1).wrapping_shl(bits - 1);
                    if extracted >= half {
                        let neg = mask - extracted;
                        return format!("-{neg}");
                    }
                }
                let i = I256::from_raw(extracted);
                format!("{i}")
            }
            Self::Uint(_) | Self::Array { .. } | Self::Mapping | Self::Struct => {
                format!("{extracted}")
            }
        }
    }
}

/// Recorded mapping slots for a single contract address.
///
/// Tracks `keccak256(key || base_slot)` results observed during execution
/// so that hashed mapping slots can be resolved back to human-readable
/// `mapping[key]` labels. Also tracks 32-byte `keccak256` results for
/// dynamic array data area starts.
#[derive(Clone, Debug, Default)]
pub struct MappingSlots {
    /// slot -> parent slot
    parent_slots: HashMap<alloy_primitives::B256, alloy_primitives::B256>,
    /// slot -> key
    keys: HashMap<alloy_primitives::B256, alloy_primitives::B256>,
    /// keccak256 result -> (key, parent)
    seen_sha3: HashMap<alloy_primitives::B256, (alloy_primitives::B256, alloy_primitives::B256)>,
    /// keccak256 results from 32-byte inputs, representing dynamic array
    /// data area starts. Maps: data_start -> parent slot (where length is).
    array_starts: HashMap<alloy_primitives::B256, alloy_primitives::B256>,
}

impl MappingSlots {
    /// Record a `keccak256(key || parent)` operation with 64-byte input.
    pub fn record_sha3(
        &mut self,
        result: alloy_primitives::B256,
        key: alloy_primitives::B256,
        parent: alloy_primitives::B256,
    ) {
        self.seen_sha3.insert(result, (key, parent));
    }

    /// Record a dynamic array data area start: `keccak256(parent_slot)`
    /// where `parent_slot` is the storage slot holding the array length.
    /// This is called for 32-byte KECCAK256 inputs.
    pub fn record_array_start(
        &mut self,
        result: alloy_primitives::B256,
        parent: alloy_primitives::B256,
    ) {
        self.array_starts.insert(result, parent);
    }

    /// Try to register a mapping slot. Returns `true` if the slot was
    /// recognised as a mapping entry.
    pub fn insert(&mut self, slot: alloy_primitives::B256) -> bool {
        let Some((key, parent)) = self.seen_sha3.get(&slot).copied() else {
            return false;
        };
        if self.keys.contains_key(&slot) {
            return false;
        }
        self.keys.insert(slot, key);
        self.parent_slots.insert(slot, parent);
        self.insert(parent);
        true
    }

    /// Try to register a mapping slot that is the base of a nearby storage
    /// access. This is needed for struct-valued mappings where the first field
    /// is never touched, so the exact base slot never appears in an `SSTORE`.
    pub fn insert_nearby(&mut self, slot: alloy_primitives::B256) -> bool {
        if self.insert(slot) {
            return true;
        }
        let slot_u256 = U256::from_be_bytes(slot.0);
        for (known_slot, (key, parent)) in &self.seen_sha3 {
            let known_u256 = U256::from_be_bytes(known_slot.0);
            if slot_u256 >= known_u256 {
                let offset = slot_u256 - known_u256;
                if offset < 128 {
                    if !self.keys.contains_key(known_slot) {
                        self.keys.insert(*known_slot, *key);
                        self.parent_slots.insert(*known_slot, *parent);
                        self.insert(*parent);
                    }
                    return true;
                }
            }
        }
        false
    }

    /// Return the chain of keys from outermost to innermost for a mapping
    /// slot, or `None` if the slot is unknown.
    pub fn key_chain(&self, slot: alloy_primitives::B256) -> Option<Vec<alloy_primitives::B256>> {
        if !self.keys.contains_key(&slot) {
            return None;
        }
        let mut current = slot;
        let mut keys = Vec::new();
        while let Some(key) = self.keys.get(&current) {
            keys.push(*key);
            let parent = self.parent_slots.get(&current)?;
            if !self.keys.contains_key(parent) {
                break;
            }
            current = *parent;
        }
        keys.reverse();
        Some(keys)
    }

    /// Return the base slot (the ultimate parent) of a mapping slot chain.
    pub fn base_slot(&self, slot: alloy_primitives::B256) -> Option<alloy_primitives::B256> {
        if !self.keys.contains_key(&slot) {
            return None;
        }
        let mut current = slot;
        while let Some(_key) = self.keys.get(&current) {
            let parent = self.parent_slots.get(&current)?;
            if !self.keys.contains_key(parent) {
                return Some(*parent);
            }
            current = *parent;
        }
        None
    }

    /// Return the parent slot (where the length is stored) for a recorded
    /// dynamic array data area start, or `None` if unknown.
    pub fn array_start_parent(
        &self,
        slot: alloy_primitives::B256,
    ) -> Option<alloy_primitives::B256> {
        self.array_starts.get(&slot).copied()
    }

    /// Iterate over all recorded array data starts.
    pub fn array_start_entries(
        &self,
    ) -> impl Iterator<Item = (&alloy_primitives::B256, &alloy_primitives::B256)> {
        self.array_starts.iter()
    }

    /// Return `true` if `slot` is a known keccak256 result with a non-zero
    /// key (i.e. a mapping slot rather than a dynamic-array data start).
    pub fn is_mapping_result(&self, slot: &alloy_primitives::B256) -> bool {
        self.seen_sha3
            .get(slot)
            .map(|(k, _)| !k.is_zero())
            .unwrap_or(false)
    }
}

/// Raw call trace tree.
///
/// Holds only the execution frames. To format a trace with address labels
/// and ABI-decoded calls, use [`Trace::display_with`] together with a
/// [`TraceContext`].
#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub roots: Vec<CallFrame>,
    pub mapping_slots: HashMap<Address, MappingSlots>,
}

impl Trace {
    /// Create a new [`Trace`] with the given roots.
    pub fn new(roots: Vec<CallFrame>) -> Self {
        Self {
            roots,
            mapping_slots: HashMap::new(),
        }
    }

    /// Return a [`Display`](fmt::Display)-able view of this trace using the
    /// given [`TraceContext`] for labels and ABI decoding.
    pub fn display_with<'a>(&'a self, ctx: &'a TraceContext) -> TraceDisplay<'a> {
        let mut labels = HashMap::new();
        for root in &self.roots {
            Self::collect_create_labels(root, ctx, &mut labels);
            Self::collect_vm_labels(root, &mut labels);
            Self::collect_code_hash_labels(root, ctx, &mut labels);
        }
        TraceDisplay {
            trace: self,
            ctx,
            labels,
        }
    }

    fn collect_vm_labels(frame: &CallFrame, labels: &mut HashMap<Address, String>) {
        const LABEL_SELECTOR: [u8; 4] = [0xc6, 0x57, 0xc7, 0x18];
        if frame.address == Some(VM_ADDRESS)
            && frame.input.len() >= 4
            && frame.input[..4] == LABEL_SELECTOR
        {
            let types = DynSolType::Tuple(vec![DynSolType::Address, DynSolType::String]);
            if let Ok(DynSolValue::Tuple(values)) = types.abi_decode_params(&frame.input[4..])
                && let (DynSolValue::Address(addr), DynSolValue::String(label)) =
                    (&values[0], &values[1])
            {
                labels.insert(*addr, label.clone());
            }
        }
        for child in &frame.children {
            Self::collect_vm_labels(child, labels);
        }
    }

    fn collect_create_labels(
        frame: &CallFrame,
        ctx: &TraceContext,
        labels: &mut HashMap<Address, String>,
    ) {
        if frame.kind == CallFrameKind::Create
            && let Some(addr) = frame.address
        {
            if let Some(name) = ctx
                .resolve_by_bytecode(&frame.output)
                .or_else(|| ctx.resolve_by_initcode(&frame.input))
            {
                labels.insert(addr, name.into());
            } else if let Some(name) = ctx.get_label(&addr) {
                labels.insert(addr, name.into());
            }
        }
        for child in &frame.children {
            Self::collect_create_labels(child, ctx, labels);
        }
    }

    /// Label addresses from bytecode stored in call frames.
    ///
    /// Libraries called via delegatecall have their bytecode stored
    /// in [`CallFrame::code_bytes`]. This pass resolves those bytecodes
    /// against the artifact index so that library names appear in traces.
    fn collect_code_hash_labels(
        frame: &CallFrame,
        ctx: &TraceContext,
        labels: &mut HashMap<Address, String>,
    ) {
        if let (Some(addr), Some(code_bytes)) = (frame.code_address, frame.code_bytes.as_ref())
            && !labels.contains_key(&addr)
            && ctx.get_label(&addr).is_none()
            && let Some(name) = ctx.resolve_by_bytecode(code_bytes)
        {
            labels.insert(addr, name.into());
        }
        for child in &frame.children {
            Self::collect_code_hash_labels(child, ctx, labels);
        }
    }
}

/// A [`Display`](fmt::Display) wrapper for a [`Trace`] backed by a
/// [`TraceContext`].
pub struct TraceDisplay<'a> {
    trace: &'a Trace,
    ctx: &'a TraceContext,
    labels: HashMap<Address, String>,
}

impl<'a> fmt::Display for TraceDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, root) in self.trace.roots.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            // Call header with counter and revert indicator.
            if root.success {
                writeln!(f, "--- Call #{} ---", i + 1)?;
            } else {
                writeln!(f, "--- Call #{} [REVERT] ---", i + 1)?;
            }
            self.write_frame(f, root, None, &[])?;
        }

        let mut logs = Vec::new();
        for root in &self.trace.roots {
            self.collect_log_events(root, &mut logs);
        }
        if !logs.is_empty() {
            writeln!(f)?;
            writeln!(f, "Logs:")?;
            for log in logs {
                writeln!(f, "  {log}")?;
            }
        }

        Ok(())
    }
}

impl<'a> TraceDisplay<'a> {
    /// Append vertical bars at `cols` (display columns), then spaces up to
    /// `target`, tracking the running display position.
    fn append_tree_prefix(buf: &mut String, cols: &[usize], target: usize) {
        let mut pos = 0;
        for &col in cols {
            buf.push_str(&" ".repeat(col.saturating_sub(pos)));
            buf.push('│');
            pos = col + 1;
        }
        buf.push_str(&" ".repeat(target.saturating_sub(pos)));
    }

    fn write_frame(
        &self,
        f: &mut fmt::Formatter<'_>,
        frame: &CallFrame,
        parent: Option<&CallFrame>,
        ancestor_cols: &[usize],
    ) -> fmt::Result {
        // Frame line prefix: vertical bars at each ancestor's children
        // column, then the branch glyph at this frame's own column (the
        // parent's children column).
        let branch_col = ancestor_cols.last().copied();
        let mut prefix = String::new();
        if let Some(col) = branch_col {
            Self::append_tree_prefix(&mut prefix, &ancestor_cols[..ancestor_cols.len() - 1], col);
            prefix.push_str("├─ ");
        }

        // Column at which this frame's name starts; children and
        // pseudo-children branch under it.
        let gas_len = frame.gas_used.to_string().len();
        let name_col = branch_col.map_or(gas_len + 3, |col| col + gas_len + 6);

        // Write the frame line
        let label = {
            // For delegatecall/callcode the executing code belongs to
            // code_address (the implementation), not address (the proxy).
            let resolve_addr = match (&frame.code_address, &frame.address) {
                (Some(ca), Some(a)) if ca != a => Some(ca),
                _ => frame.address.as_ref(),
            };
            resolve_addr.map(|addr| {
                self.labels
                    .get(addr)
                    .map(|s| s.as_str())
                    .or_else(|| self.ctx.get_label(addr))
                    .map(|s| s.into())
                    .unwrap_or_else(|| addr.to_checksum(None))
            })
        };

        if matches!(frame.kind, CallFrameKind::Create) {
            let label = self
                .ctx
                .resolve_by_bytecode(&frame.output)
                .or_else(|| {
                    frame
                        .address
                        .and_then(|addr| self.labels.get(&addr).map(|s| s.as_str()))
                })
                .or_else(|| frame.address.and_then(|addr| self.ctx.get_label(&addr)))
                .unwrap_or("<unknown>");
            let addr = frame
                .address
                .map(|a| format!("{a}"))
                .unwrap_or_else(|| "unknown".into());
            writeln!(f, "{prefix}[{}] → new {label}@{addr}", frame.gas_used)?;
        } else {
            let addr = label.as_deref().unwrap_or("unknown");
            let selector = if frame.input.len() >= 4 {
                format!("0x{}", hex::encode(&frame.input[..4]))
            } else {
                format!("0x{}", hex::encode(&frame.input))
            };
            let (func_name, args) = self.ctx.decode_call(&frame.input);
            let func_name = func_name.unwrap_or(&selector);
            let scheme_suffix = match frame.kind {
                CallFrameKind::Call(CallScheme::StaticCall) => " [staticcall]",
                CallFrameKind::Call(CallScheme::DelegateCall) => " [delegatecall]",
                CallFrameKind::Call(CallScheme::CallCode) => " [callcode]",
                _ => "",
            };
            writeln!(
                f,
                "{prefix}[{}] {addr}::{func_name}({args}){scheme_suffix}",
                frame.gas_used
            )?;
        }

        // Build prefixes for call context / call context changes
        let mut meta_prefix = String::new();
        Self::append_tree_prefix(&mut meta_prefix, ancestor_cols, name_col);
        meta_prefix.push_str("├─ ");

        let mut meta_detail_prefix = String::new();
        Self::append_tree_prefix(&mut meta_detail_prefix, ancestor_cols, name_col);
        meta_detail_prefix.push_str("│   ");

        // Compute which fields differ from the parent (if any).
        match parent {
            None => {
                // Root frame: show full call context.
                writeln!(f, "{meta_prefix} call context:")?;
                let caller_label = self
                    .labels
                    .get(&frame.caller)
                    .map(|s| s.as_str())
                    .or_else(|| self.ctx.get_label(&frame.caller))
                    .map(|s| format!("{s} [{}]", frame.caller.to_checksum(None)))
                    .unwrap_or_else(|| frame.caller.to_checksum(None));
                writeln!(f, "{meta_detail_prefix}@ msg.sender: {caller_label}")?;
                writeln!(f, "{meta_detail_prefix}@ msg.value: {}", frame.value)?;
                writeln!(
                    f,
                    "{meta_detail_prefix}@ block.timestamp: {}",
                    frame.timestamp
                )?;
                writeln!(f, "{meta_detail_prefix}@ block.number: {}", frame.number)?;
            }
            Some(parent_frame) => {
                // Child frame: only show fields that differ from the parent.
                let sender_diff = frame.caller != parent_frame.caller;
                let value_diff = frame.value != parent_frame.value;
                let timestamp_diff = frame.timestamp != parent_frame.timestamp;
                let number_diff = frame.number != parent_frame.number;
                let any_diff = sender_diff || value_diff || timestamp_diff || number_diff;

                if any_diff {
                    writeln!(f, "{meta_prefix} call context changes:")?;
                    if sender_diff {
                        let caller_label = self
                            .labels
                            .get(&frame.caller)
                            .map(|s| s.as_str())
                            .or_else(|| self.ctx.get_label(&frame.caller))
                            .map(|s| format!("{s} [{}]", frame.caller.to_checksum(None)))
                            .unwrap_or_else(|| frame.caller.to_checksum(None));
                        writeln!(f, "{meta_detail_prefix}@ msg.sender: {caller_label}")?;
                    }
                    if value_diff {
                        writeln!(f, "{meta_detail_prefix}@ msg.value: {}", frame.value)?;
                    }
                    if timestamp_diff {
                        writeln!(
                            f,
                            "{meta_detail_prefix}@ block.timestamp: {}",
                            frame.timestamp
                        )?;
                    }
                    if number_diff {
                        writeln!(f, "{meta_detail_prefix}@ block.number: {}", frame.number)?;
                    }
                }
            }
        }

        // Write children
        let mut child_cols = ancestor_cols.to_vec();
        child_cols.push(name_col);
        for child in &frame.children {
            self.write_frame(f, child, Some(frame), &child_cols)?;
        }

        // Write logs as pseudo-children
        for log in &frame.logs {
            if self.ctx.decode_log_event(log).is_some() {
                continue;
            }
            let mut log_prefix = String::new();
            Self::append_tree_prefix(&mut log_prefix, ancestor_cols, name_col);
            log_prefix.push_str("├─ ");
            let (name, args) = self.ctx.decode_event(log);
            let name = name.as_deref().unwrap_or("Log");
            writeln!(f, "{log_prefix}emit {name}({args})")?;
        }

        // Write storage changes as pseudo-children
        let actual_changes: Vec<&StorageChange> = frame
            .storage_changes
            .iter()
            .filter(|c| c.old_value != c.new_value)
            .collect();
        if !actual_changes.is_empty() {
            let mut storage_prefix = String::new();
            Self::append_tree_prefix(&mut storage_prefix, ancestor_cols, name_col);
            storage_prefix.push_str("├─ ");
            writeln!(f, "{storage_prefix} storage changes:")?;

            let mut change_prefix = String::new();
            Self::append_tree_prefix(&mut change_prefix, ancestor_cols, name_col);
            change_prefix.push_str("│   ");

            for change in actual_changes {
                let label = frame.address.and_then(|addr| {
                    self.labels
                        .get(&addr)
                        .map(|s| s.as_str())
                        .or_else(|| self.ctx.get_label(&addr))
                });
                // Resolve the artifact name for storage layout lookup.
                // Walk the frame hierarchy: delegatecall through libraries
                // means the executing bytecode may not match the
                // storage-owning contract.
                let storage_contract_name = {
                    let mut name = frame
                        .code_bytes
                        .as_ref()
                        .and_then(|b| self.ctx.resolve_by_bytecode(b))
                        .filter(|n| self.ctx.has_storage(n));
                    if name.is_none() {
                        name = parent
                            .and_then(|p| p.code_bytes.as_ref())
                            .and_then(|b| self.ctx.resolve_by_bytecode(b))
                            .filter(|n| self.ctx.has_storage(n));
                    }
                    name
                };
                let mapping_slots = frame
                    .address
                    .and_then(|addr| self.trace.mapping_slots.get(&addr));
                let packed_changes = storage_contract_name
                    .or(label)
                    .map(|n| {
                        self.ctx.resolve_storage_changes(
                            n,
                            &change.slot,
                            change.old_value,
                            change.new_value,
                            mapping_slots,
                        )
                    })
                    .unwrap_or_default();

                if packed_changes.is_empty() {
                    let (name, ty) = storage_contract_name
                        .or(label)
                        .and_then(|n| {
                            let name =
                                self.ctx
                                    .resolve_storage_name(n, &change.slot, mapping_slots)?;
                            let ty = self
                                .ctx
                                .resolve_storage_type(n, &change.slot, mapping_slots);
                            Some((name, ty))
                        })
                        .unwrap_or_else(|| (format!("{}", change.slot), None));
                    let old = ty
                        .map(|t| t.format_value(change.old_value, 0, 32))
                        .unwrap_or_else(|| format!("{}", change.old_value));
                    let new = ty
                        .map(|t| t.format_value(change.new_value, 0, 32))
                        .unwrap_or_else(|| format!("{}", change.new_value));
                    writeln!(f, "{change_prefix}@ {name}: {old} -> {new}")?;
                } else {
                    for info in packed_changes {
                        let old = info
                            .ty
                            .format_value(change.old_value, info.offset, info.bytes);
                        let new = info
                            .ty
                            .format_value(change.new_value, info.offset, info.bytes);
                        writeln!(f, "{change_prefix}@ {}: {old} -> {new}", info.name)?;
                    }
                }
            }
        }

        // Write result as a pseudo-child
        let mut result_prefix = String::new();
        Self::append_tree_prefix(&mut result_prefix, ancestor_cols, name_col);
        result_prefix.push_str("└─ ");

        if frame.success {
            if frame.kind == CallFrameKind::Create {
                let code_len = frame.output.len();
                if code_len > 0 {
                    writeln!(f, "{result_prefix}← [return] {code_len} bytes of code")?;
                } else {
                    writeln!(f, "{result_prefix}← [stop]")?;
                }
            } else {
                let decoded = self.ctx.decode_return(&frame.input, &frame.output);
                let out = if let Some(decoded) = decoded {
                    decoded
                } else if frame.output.is_empty() {
                    String::new()
                } else {
                    format!("0x{}", hex::encode(&frame.output))
                };
                if out.is_empty() {
                    if is_empty_code_call(frame) {
                        writeln!(f, "{result_prefix}← [stop] (no code)")?;
                    } else {
                        writeln!(f, "{result_prefix}← [stop]")?;
                    }
                } else {
                    writeln!(f, "{result_prefix}← [return] {out}")?;
                }
            }
        } else {
            let revert = empty_code_revert_reason(frame)
                .unwrap_or_else(|| self.ctx.decode_revert(&frame.output));
            writeln!(f, "{result_prefix}← [revert] {revert}")?;
        }

        Ok(())
    }

    fn collect_log_events(&self, frame: &CallFrame, logs: &mut Vec<String>) {
        for log in &frame.logs {
            if let Some(msg) = self.ctx.decode_log_event(log) {
                logs.push(msg);
            }
        }
        for child in &frame.children {
            self.collect_log_events(child, logs);
        }
    }
}

/// The kind of a call frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallFrameKind {
    Call(CallScheme),
    Create,
}

/// True when this call frame targeted an account with no bytecode.
///
/// Precompiles execute without account code, so they are excluded.
fn is_empty_code_call(frame: &CallFrame) -> bool {
    if !matches!(frame.kind, CallFrameKind::Call(_)) {
        return false;
    }
    if frame.code_bytes.is_some() {
        return false;
    }
    let Some(addr) = frame.address.or(frame.code_address) else {
        return false;
    };
    !revm::precompile::Precompiles::latest().contains(&addr)
}

/// Prefer a clear reason when an empty revert follows a call to an empty account.
///
/// Solidity high-level calls to addresses with no code succeed at the EVM level
/// (empty returndata / STOP) and then the caller reverts with empty data. Without
/// this, traces only show `reverted`, which hides the missing-contract cause.
fn empty_code_revert_reason(frame: &CallFrame) -> Option<String> {
    if !frame.output.is_empty() {
        return None;
    }
    // Prefer the most recent empty-code child as the likely cause.
    frame.children.iter().rev().find_map(|child| {
        if !is_empty_code_call(child) {
            return None;
        }
        let addr = child.address.or(child.code_address)?;
        Some(format!("no contract code at {}", addr.to_checksum(None)))
    })
}

/// A single frame in a raw call trace.
#[derive(Debug, Clone)]
pub struct CallFrame {
    pub depth: usize,
    pub kind: CallFrameKind,
    /// The execution context address (e.g. the proxy for a delegatecall).
    pub address: Option<Address>,
    /// The address whose code is executing.
    /// Differs from `address` for delegatecall / callcode (proxy patterns).
    pub code_address: Option<Address>,
    /// Raw bytecode at `code_address`, captured for contract-name resolution.
    /// Used to resolve library names when the address is not labelled.
    pub code_bytes: Option<Bytes>,
    pub caller: Address,
    pub value: U256,
    pub timestamp: U256,
    pub number: U256,
    pub input: Bytes,
    pub output: Bytes,
    pub gas_used: u64,
    pub success: bool,
    pub children: Vec<CallFrame>,
    pub storage_changes: Vec<StorageChange>,
    pub logs: Vec<Log>,
}
