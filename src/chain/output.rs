//! Execution output types for the chain abstraction.

use crate::chain::inspectors::trace::TraceTree;
use crate::corpus::LocalCoverage;

/// Result of executing a call sequence against a chain.
#[derive(Debug)]
pub struct ExecutionOutput {
    pub coverage: LocalCoverage,
    pub trace: Option<TraceTree>,
    pub call_meta: Vec<CallMeta>,
    pub property_results: Vec<PropertyResult>,
    pub all_ok: bool,
    /// Number of individual calls executed in this sequence (including calls that reverted).
    pub total_calls: u64,
    /// Total gas consumed by all calls in this sequence.
    pub total_gas: u64,
}

/// Result of checking a single property function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyResult {
    pub name: String,
    pub selector: [u8; 4],
    pub passed: bool,
}

/// Metadata for a single call in an executed sequence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CallMeta {
    /// Block number at execution time.
    pub block_number: u64,
    /// Block timestamp at execution time.
    pub block_timestamp: u64,
}
