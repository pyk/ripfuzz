//! Raw call trace types for the EVM chain.

use std::collections::HashMap;
use std::fmt;

use alloy_json_abi::JsonAbi;
use alloy_primitives::FixedBytes;
use alloy_sol_types::SolError;
use revm::primitives::{Address, Bytes};

pub use inspector::Inspector;

mod inspector;

/// Raw call trace tree.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub roots: Vec<CallFrame>,
    pub labels: HashMap<Address, String>,
    pub abi: Option<JsonAbi>,
}

impl Trace {
    /// Create a new [`Trace`] with the given roots.
    pub fn new(roots: Vec<CallFrame>) -> Self {
        Self {
            roots,
            labels: HashMap::new(),
            abi: None,
        }
    }

    /// Set an address label for formatting.
    pub fn with_label(mut self, address: Address, label: impl Into<String>) -> Self {
        self.labels.insert(address, label.into());
        self
    }

    /// Set an ABI for decoding function calls and revert reasons.
    pub fn with_abi(mut self, abi: JsonAbi) -> Self {
        self.abi = Some(abi);
        self
    }

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
                .cloned()
                .unwrap_or_else(|| format!("{addr:#x}"))
        });

        if frame.kind == CallFrameKind::Create {
            let label = label.as_deref().unwrap_or("<unknown>");
            let addr = frame
                .address
                .map(|a| format!("{a:#x}"))
                .unwrap_or_else(|| "unknown".into());
            writeln!(f, "{prefix}[{}] → new {label}@{addr}", frame.gas_used)?;
        } else {
            let addr = label.as_deref().unwrap_or("unknown");
            let selector = if frame.input.len() >= 4 {
                format!("0x{}", hex::encode(&frame.input[..4]))
            } else {
                format!("0x{}", hex::encode(&frame.input))
            };
            let func_name = self
                .abi
                .as_ref()
                .and_then(|abi| {
                    if frame.input.len() >= 4 {
                        let sel = FixedBytes::new(frame.input[..4].try_into().ok()?);
                        abi.function_by_selector(sel).map(|func| &func.name[..])
                    } else {
                        None
                    }
                })
                .unwrap_or(&selector);
            writeln!(f, "{prefix}[{}] {addr}::{func_name}(...)", frame.gas_used)?;
        }

        // Build has_next for children
        let mut child_has_next = has_next.to_vec();
        child_has_next.push(!is_last);

        // Write children
        for child in &frame.children {
            self.write_frame(f, child, &child_has_next, false)?;
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
                    writeln!(f, "{result_prefix}← [Return] {code_len} bytes of code")?;
                } else {
                    writeln!(f, "{result_prefix}← [Stop]")?;
                }
            } else {
                let out = if frame.output.is_empty() {
                    String::new()
                } else {
                    format!("0x{}", hex::encode(&frame.output))
                };
                if out.is_empty() {
                    writeln!(f, "{result_prefix}← [Stop]")?;
                } else {
                    writeln!(f, "{result_prefix}← [Return] {out}")?;
                }
            }
        } else {
            let revert = self.decode_revert(&frame.output);
            writeln!(f, "{result_prefix}← [Revert] {revert}")?;
        }

        Ok(())
    }

    fn decode_revert(&self, data: &Bytes) -> String {
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
        if let Some(abi) = &self.abi
            && data.len() >= 4
        {
            let sel: FixedBytes<4> = FixedBytes::new(data[..4].try_into().unwrap_or_default());
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

        format!("0x{}", hex::encode(data))
    }
}

impl fmt::Display for Trace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for root in &self.roots {
            self.write_frame(f, root, &[], true)?;
        }
        Ok(())
    }
}

/// The kind of a call frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallFrameKind {
    Call,
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
}
