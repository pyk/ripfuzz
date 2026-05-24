//! Raw call trace types for the EVM chain.

use revm::primitives::{Address, Bytes};

pub use inspector::Inspector;
pub use viewer::Viewer;

pub mod inspector;
pub mod viewer;

/// Raw call trace tree.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub roots: Vec<CallFrame>,
}

/// A single frame in a raw call trace.
#[derive(Debug, Clone)]
pub struct CallFrame {
    pub depth: usize,
    pub address: Option<Address>,
    pub input: Bytes,
    pub output: Bytes,
    pub gas_used: u64,
    pub success: bool,
    pub children: Vec<CallFrame>,
}
