//! Raw call trace types for the EVM chain.

use std::collections::HashMap;
use std::fmt;

use revm::interpreter::CallScheme;
use revm::primitives::{Address, Bytes};

pub use context::TraceContext;
pub use inspector::Inspector;

mod context;
mod inspector;

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
        }
        TraceDisplay {
            trace: self,
            ctx,
            labels,
        }
    }

    fn collect_create_labels(
        frame: &CallFrame,
        ctx: &TraceContext,
        labels: &mut HashMap<Address, String>,
    ) {
        if frame.kind == CallFrameKind::Create
            && let Some(name) = ctx.resolve_by_bytecode(&frame.output)
            && let Some(addr) = frame.address
        {
            labels.insert(addr, name.into());
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
                .or_else(|| frame.address.and_then(|addr| self.ctx.get_label(&addr)))
                .or_else(|| {
                    frame
                        .address
                        .and_then(|addr| self.labels.get(&addr).map(|s| s.as_str()))
                })
                .unwrap_or("<unknown>");
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
            let revert = self.ctx.decode_revert(&frame.output);
            writeln!(f, "{result_prefix}← [Revert] {revert}")?;
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
}
