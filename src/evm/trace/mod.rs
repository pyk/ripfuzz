//! Raw call trace types for the EVM chain.

pub mod inspector;
pub mod viewer;

use revm::primitives::{Address, Bytes};

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

pub use inspector::Inspector;
pub use viewer::Viewer;
