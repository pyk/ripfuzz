//! Raw call trace types for the EVM chain.

use std::collections::HashMap;
use std::fmt;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{I256, U256};
use revm::interpreter::CallScheme;
use revm::primitives::{Address, Bytes};

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
                format!("0x{}", hex::encode(&bytes[12..]))
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

/// Raw call trace tree.
///
/// Holds only the execution frames. To format a trace with address labels
/// and ABI-decoded calls, use [`Trace::display_with`] together with a
/// [`TraceContext`].
#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub roots: Vec<CallFrame>,
}

impl Trace {
    /// Create a new [`Trace`] with the given roots.
    pub fn new(roots: Vec<CallFrame>) -> Self {
        Self { roots }
    }

    /// Return a [`Display`](fmt::Display)-able view of this trace using the
    /// given [`TraceContext`] for labels and ABI decoding.
    pub fn display_with<'a>(&'a self, ctx: &'a TraceContext) -> TraceDisplay<'a> {
        let mut labels = HashMap::new();
        for root in &self.roots {
            Self::collect_create_labels(root, ctx, &mut labels);
            Self::collect_vm_labels(root, &mut labels);
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
        for root in &self.trace.roots {
            self.write_frame(f, root, &[], true)?;
        }
        Ok(())
    }
}

impl<'a> TraceDisplay<'a> {
    fn write_frame(
        &self,
        f: &mut fmt::Formatter<'_>,
        frame: &CallFrame,
        has_next: &[bool],
        is_last: bool,
    ) -> fmt::Result {
        // Build the prefix string for the frame line
        let mut prefix = String::new();
        if !has_next.is_empty() {
            for h in has_next {
                if *h {
                    prefix.push_str("│   ");
                } else {
                    prefix.push_str("    ");
                }
            }
            if is_last {
                prefix.push_str("└─ ");
            } else {
                prefix.push_str("├─ ");
            }
        }

        // Write the frame line
        let label = frame.address.map(|addr| {
            self.labels
                .get(&addr)
                .map(|s| s.as_str())
                .or_else(|| self.ctx.get_label(&addr))
                .map(|s| s.into())
                .unwrap_or_else(|| format!("{addr:#x}"))
        });

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
                _ => "",
            };
            writeln!(
                f,
                "{prefix}[{}] {addr}::{func_name}({args}){scheme_suffix}",
                frame.gas_used
            )?;
        }

        // Build has_next for children
        let mut child_has_next = has_next.to_vec();
        child_has_next.push(!is_last);

        // Write children
        for child in &frame.children {
            self.write_frame(f, child, &child_has_next, false)?;
        }

        // Write storage changes as pseudo-children
        let actual_changes: Vec<&StorageChange> = frame
            .storage_changes
            .iter()
            .filter(|c| c.old_value != c.new_value)
            .collect();
        if !actual_changes.is_empty() {
            let mut storage_prefix = String::new();
            for h in &child_has_next {
                if *h {
                    storage_prefix.push_str("│   ");
                } else {
                    storage_prefix.push_str("    ");
                }
            }
            storage_prefix.push_str("├─ ");
            writeln!(f, "{storage_prefix} storage changes:")?;

            let mut change_prefix = String::new();
            for h in &child_has_next {
                if *h {
                    change_prefix.push_str("│   ");
                } else {
                    change_prefix.push_str("    ");
                }
            }
            change_prefix.push_str("│   ");

            for change in actual_changes {
                let label = frame.address.and_then(|addr| {
                    self.labels
                        .get(&addr)
                        .map(|s| s.as_str())
                        .or_else(|| self.ctx.get_label(&addr))
                });
                let packed_changes = label
                    .map(|l| {
                        self.ctx.resolve_storage_changes(
                            l,
                            &change.slot,
                            change.old_value,
                            change.new_value,
                        )
                    })
                    .unwrap_or_default();

                if packed_changes.is_empty() {
                    let (name, ty) = label
                        .and_then(|l| {
                            let name = self.ctx.resolve_storage_name(l, &change.slot)?;
                            let ty = self.ctx.resolve_storage_type(l, &change.slot);
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
        for h in &child_has_next {
            if *h {
                result_prefix.push_str("│   ");
            } else {
                result_prefix.push_str("    ");
            }
        }
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
                let out = if frame.output.is_empty() {
                    String::new()
                } else {
                    format!("0x{}", hex::encode(&frame.output))
                };
                if out.is_empty() {
                    writeln!(f, "{result_prefix}← [stop]")?;
                } else {
                    writeln!(f, "{result_prefix}← [return] {out}")?;
                }
            }
        } else {
            let revert = self.ctx.decode_revert(&frame.output);
            writeln!(f, "{result_prefix}← [revert] {revert}")?;
        }

        Ok(())
    }
}

/// The kind of a call frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallFrameKind {
    Call(CallScheme),
    Create,
}

/// A single frame in a raw call trace.
#[derive(Debug, Clone)]
pub struct CallFrame {
    pub depth: usize,
    pub kind: CallFrameKind,
    pub address: Option<Address>,
    pub input: Bytes,
    pub output: Bytes,
    pub gas_used: u64,
    pub success: bool,
    pub children: Vec<CallFrame>,
    pub storage_changes: Vec<StorageChange>,
}
