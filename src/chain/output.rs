//! Execution output types for the chain abstraction.

use crate::chain::inspectors::trace::TraceTree;
use crate::corpus::LocalCoverage;

/// Result of executing a call sequence against a chain.
#[derive(Debug)]
pub struct ExecutionOutput {
    pub coverage: LocalCoverage,
    pub trace: Option<TraceTree>,
    pub call_meta: Vec<CallMeta>,
    pub all_ok: bool,
    /// Number of individual calls executed in this sequence (including calls that reverted).
    pub total_calls: u64,
    /// Total gas consumed by all calls in this sequence.
    pub total_gas: u64,
    /// If an assert panic was detected, details about the crash.
    pub crash: Option<CrashInfo>,
}

/// Details about an assert panic crash detected during execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashInfo {
    pub name: String,
    pub selector: [u8; 4],
}

fn default_true() -> bool {
    true
}

/// Metadata for a single call in an executed sequence.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CallMeta {
    /// Block number at execution time.
    pub block_number: u64,
    /// Block timestamp at execution time.
    pub block_timestamp: u64,
    /// Gas consumed by this individual call.
    #[serde(default)]
    pub gas_used: u64,
    /// Whether this call succeeded.
    #[serde(default = "default_true")]
    pub success: bool,
    /// If the call reverted or halted, the human-readable reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
