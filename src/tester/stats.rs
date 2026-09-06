//! Fuzzing statistics for a test campaign.
//!
//! [`SharedStats`] aggregates per-function counters across fuzzer threads while
//! [`Stats`] is the serializable snapshot saved under `{root}/.ripfuzz/stats`.
//! [`StatsWriter`] performs the file write so aggregation stays free of I/O.
//!
//! ```rust,no_run
//! use ripfuzz::tester::{SharedStats, Stats, StatsMetadata, StatsWriter};
//!
//! # let handlers_stats = Vec::new();
//! # let invariants_stats = Vec::new();
//! # let metadata = StatsMetadata {
//! #     harness: String::new(), address: String::new(), chain_id: 0, seed: 0,
//! #     threads: 0, max_runs: 0, max_calls: 0, timeout_secs: None,
//! #     duration_secs: 0.0, total_sequences: 0, total_handler_calls: 0,
//! #     total_invariant_checks: 0, broken_invariants: 0,
//! #     rpc: ripfuzz::tester::RpcSummary::new(),
//! # };
//! let stats = Stats::new()
//!     .with_metadata(metadata)
//!     .with_handlers_stats(handlers_stats)
//!     .with_invariants_stats(invariants_stats);
//! let path = StatsWriter::new()
//!     .with_root(std::path::Path::new("."))
//!     .with_stats(stats)
//!     .write()
//!     .unwrap();
//! println!("statistics: {}", path.display());
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf, absolute};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use alloy_json_abi::{Function, JsonAbi};
use alloy_sol_types::SolError;
use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::cli::RunId;
use crate::compilers::solc::SolcOutput;
use crate::evm::TransactionResult;

alloy_sol_types::sol! {
    error Error(string message);
    error Panic(uint256 code);
    error BrokenInvariantError(string id, string description);
}

/// One grouped revert with its call count.
///
/// Each kind carries only its own data:
///
/// - `Error` holds the selector and the decoded string
/// - `Panic` holds the selector and the code as hex
/// - `CustomError` holds the selector and the resolved error name
/// - `BrokenInvariantError` holds the selector and `{id}: {description}`
/// - `UnknownRevert` holds the raw output as `0x` prefixed hex
/// - `EmptyRevert` and `Halt` hold no data
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RevertSummary {
    Error {
        selector: String,
        message: String,
        count: u64,
    },
    Panic {
        selector: String,
        code: String,
        count: u64,
    },
    CustomError {
        selector: String,
        name: String,
        count: u64,
    },
    BrokenInvariantError {
        selector: String,
        message: String,
        count: u64,
    },
    UnknownRevert {
        message: String,
        count: u64,
    },
    EmptyRevert {
        count: u64,
    },
    Halt {
        count: u64,
    },
}

impl RevertSummary {
    /// The revert kind, one of `Error`, `Panic`, `CustomError`,
    /// `BrokenInvariantError`, `UnknownRevert`, `EmptyRevert`, or `Halt`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Error { .. } => "Error",
            Self::Panic { .. } => "Panic",
            Self::CustomError { .. } => "CustomError",
            Self::BrokenInvariantError { .. } => "BrokenInvariantError",
            Self::UnknownRevert { .. } => "UnknownRevert",
            Self::EmptyRevert { .. } => "EmptyRevert",
            Self::Halt { .. } => "Halt",
        }
    }

    /// The 4-byte selector as `0x` prefixed hex, when the kind carries one.
    pub fn selector(&self) -> Option<&str> {
        match self {
            Self::Error { selector, .. }
            | Self::Panic { selector, .. }
            | Self::CustomError { selector, .. }
            | Self::BrokenInvariantError { selector, .. } => Some(selector),
            Self::UnknownRevert { .. } | Self::EmptyRevert { .. } | Self::Halt { .. } => None,
        }
    }

    /// The resolved custom error name, when the kind carries one.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::CustomError { name, .. } => Some(name),
            _ => None,
        }
    }

    /// The decoded payload, when the kind carries one.
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Error { message, .. }
            | Self::BrokenInvariantError { message, .. }
            | Self::UnknownRevert { message, .. } => Some(message),
            _ => None,
        }
    }

    /// The panic code as hex, when the revert is a panic.
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Panic { code, .. } => Some(code),
            _ => None,
        }
    }

    /// The number of calls that reverted with this kind and data.
    pub fn count(&self) -> u64 {
        match self {
            Self::Error { count, .. }
            | Self::Panic { count, .. }
            | Self::CustomError { count, .. }
            | Self::BrokenInvariantError { count, .. }
            | Self::UnknownRevert { count, .. }
            | Self::EmptyRevert { count, .. }
            | Self::Halt { count, .. } => *count,
        }
    }

    /// Build a summary from a grouping key and its call count.
    fn from_key(key: &RevertKey, count: u64) -> Self {
        // 1. Copy the key data into the matching variant.
        match key {
            RevertKey::Error { selector, message } => Self::Error {
                selector: selector.clone(),
                message: message.clone(),
                count,
            },
            RevertKey::Panic { selector, code } => Self::Panic {
                selector: selector.clone(),
                code: code.clone(),
                count,
            },
            RevertKey::CustomError { selector } => Self::CustomError {
                selector: selector.clone(),
                name: String::new(),
                count,
            },
            RevertKey::BrokenInvariantError { selector, message } => Self::BrokenInvariantError {
                selector: selector.clone(),
                message: message.clone(),
                count,
            },
            RevertKey::UnknownRevert { message } => Self::UnknownRevert {
                message: message.clone(),
                count,
            },
            RevertKey::EmptyRevert => Self::EmptyRevert { count },
            RevertKey::Halt => Self::Halt { count },
        }
    }
}

/// Grouping key for revert counts, one variant per revert kind without the count.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RevertKey {
    Error { selector: String, message: String },
    Panic { selector: String, code: String },
    CustomError { selector: String },
    BrokenInvariantError { selector: String, message: String },
    UnknownRevert { message: String },
    EmptyRevert,
    Halt,
}

/// Selector to error name lookup built from compilation output.
///
/// [`ErrorResolver::from_solc_output`] collects every custom error across all
/// compiled contracts so statistics can resolve a revert selector to its
/// Solidity name without touching fuzzer logic.
///
/// ```rust,no_run
/// use ripfuzz::tester::ErrorResolver;
///
/// # let solc_output: ripfuzz::compilers::solc::SolcOutput = todo!();
/// let error_resolver = ErrorResolver::from_solc_output(&solc_output);
/// println!("{:?}", error_resolver.name("0x77c522ea"));
/// ```
#[derive(Debug, Clone, Default)]
pub struct ErrorResolver {
    names: HashMap<String, String>,
}

impl ErrorResolver {
    /// Create an empty resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a resolver from every custom error in the compilation output.
    pub fn from_solc_output(output: &SolcOutput) -> Self {
        // 1. Walk every contract in the compilation unit.
        let mut error_resolver = Self::new();
        for contracts in output.output.contracts.values() {
            for contract in contracts.values() {
                // 2. Skip contracts without an ABI.
                let Some(abi) = contract.abi.as_ref() else {
                    continue;
                };
                // 3. Convert the solc ABI into the alloy representation.
                //    Contracts without a decodable ABI are skipped.
                let json = serde_json::to_value(&abi.items).unwrap_or_default();
                let abi: JsonAbi = serde_json::from_value(json).unwrap_or_default();
                error_resolver = error_resolver.with_abi(abi);
            }
        }
        error_resolver
    }

    /// Add every custom error from one contract ABI.
    pub fn with_abi(mut self, abi: JsonAbi) -> Self {
        // 1. Consume the ABI so error names move without cloning.
        for errors in abi.errors.into_values() {
            for error in errors {
                // 2. Keep the first name for a selector.
                //    The same selector implies the same signature.
                let selector = format!("0x{}", hex::encode(error.selector().as_slice()));
                self.names.entry(selector).or_insert(error.name);
            }
        }
        self
    }

    /// Resolve a selector to its error name.
    pub fn name(&self, selector: &str) -> Option<&str> {
        self.names.get(selector).map(String::as_str)
    }

    /// Fill the resolved error name for every custom error entry.
    pub fn resolve(&self, stats: &mut [FunctionStats]) {
        // 1. Walk every grouped revert in every function entry.
        for entry in stats {
            for revert in &mut entry.reverts {
                // 2. Resolve custom errors with an empty name only.
                //    Unknown selectors keep an empty name.
                if let RevertSummary::CustomError { selector, name, .. } = revert
                    && name.is_empty()
                    && let Some(resolved) = self.name(selector)
                {
                    *name = resolved.to_owned();
                }
            }
        }
    }
}

/// Wall time spent executing one function, in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WallTime {
    min_ns: u64,
    max_ns: u64,
    avg_ns: u64,
}

impl WallTime {
    /// Create wall time statistics from nanosecond measurements.
    pub fn new(min_ns: u64, max_ns: u64, avg_ns: u64) -> Self {
        Self {
            min_ns,
            max_ns,
            avg_ns,
        }
    }

    /// The fastest call in nanoseconds, zero when nothing was recorded.
    pub fn min_ns(&self) -> u64 {
        self.min_ns
    }

    /// The slowest call in nanoseconds, zero when nothing was recorded.
    pub fn max_ns(&self) -> u64 {
        self.max_ns
    }

    /// The mean call time in nanoseconds, zero when nothing was recorded.
    pub fn avg_ns(&self) -> u64 {
        self.avg_ns
    }
}

/// RPC cache usage attributed to one function.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcSummary {
    hits: u64,
    misses: u64,
    wait_ns: u64,
}

impl RpcSummary {
    /// Create an RPC summary from cache counters and wait time.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the cache hits.
    pub fn with_hits(mut self, hits: u64) -> Self {
        self.hits = hits;
        self
    }

    /// Set the cache misses.
    pub fn with_misses(mut self, misses: u64) -> Self {
        self.misses = misses;
        self
    }

    /// Set the time spent in the RPC batch path, in nanoseconds.
    pub fn with_wait_ns(mut self, wait_ns: u64) -> Self {
        self.wait_ns = wait_ns;
        self
    }

    /// Requests served from the in-memory or disk cache.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Requests that required an RPC fetch.
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Time spent in the RPC batch path, in nanoseconds.
    pub fn wait_ns(&self) -> u64 {
        self.wait_ns
    }
}

/// Aggregated statistics for one handler or invariant function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionStats {
    name: String,
    selector: String,
    calls: u64,
    successful_calls: u64,
    revert_calls: u64,
    wall_time_ns: WallTime,
    rpc: RpcSummary,
    reverts: Vec<RevertSummary>,
}

impl FunctionStats {
    /// The function name from the harness ABI.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The function selector as `0x` prefixed hex.
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// The total number of calls executed.
    pub fn calls(&self) -> u64 {
        self.calls
    }

    /// The number of calls that did not revert.
    pub fn successful_calls(&self) -> u64 {
        self.successful_calls
    }

    /// The number of calls that reverted or halted.
    pub fn revert_calls(&self) -> u64 {
        self.revert_calls
    }

    /// The wall time statistics in nanoseconds.
    pub fn wall_time_ns(&self) -> WallTime {
        self.wall_time_ns
    }

    /// The RPC cache usage attributed to this function.
    pub fn rpc(&self) -> RpcSummary {
        self.rpc
    }

    /// Reverts grouped by kind and data, most frequent first.
    pub fn reverts(&self) -> &[RevertSummary] {
        &self.reverts
    }
}

/// Campaign configuration and totals for a fuzzing statistics report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsMetadata {
    /// The fuzzed harness contract name.
    pub harness: String,
    /// The deployed harness address.
    pub address: String,
    /// The chain id the campaign ran on.
    pub chain_id: u64,
    /// The RNG seed of the campaign.
    pub seed: u64,
    /// The number of fuzzer threads.
    pub threads: usize,
    /// The maximum number of sequences across all threads.
    pub max_runs: u64,
    /// The maximum number of handler calls per sequence.
    pub max_calls: usize,
    /// The campaign timeout in seconds, when set.
    pub timeout_secs: Option<u64>,
    /// The fuzzing wall time in seconds.
    pub duration_secs: f64,
    /// The number of sequences executed.
    pub total_sequences: u64,
    /// The number of handler calls executed.
    pub total_handler_calls: u64,
    /// The number of invariant checks executed.
    pub total_invariant_checks: u64,
    /// The number of distinct broken invariants found.
    pub broken_invariants: usize,
    /// The RPC cache usage of the fuzzing phase.
    pub rpc: RpcSummary,
}

/// The serializable fuzzing statistics report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    metadata: Option<StatsMetadata>,
    handlers: Vec<FunctionStats>,
    invariants: Vec<FunctionStats>,
}

impl Stats {
    /// Create an empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the campaign configuration and totals.
    pub fn with_metadata(mut self, metadata: StatsMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Set the per-handler statistics in harness order.
    pub fn with_handlers_stats(mut self, handlers: Vec<FunctionStats>) -> Self {
        self.handlers = handlers;
        self
    }

    /// Set the per-invariant statistics in harness order.
    pub fn with_invariants_stats(mut self, invariants: Vec<FunctionStats>) -> Self {
        self.invariants = invariants;
        self
    }

    /// The campaign configuration and totals, when set.
    pub fn metadata(&self) -> Option<&StatsMetadata> {
        self.metadata.as_ref()
    }

    /// Per-handler statistics in harness order.
    pub fn handlers(&self) -> &[FunctionStats] {
        &self.handlers
    }

    /// Per-invariant statistics in harness order.
    pub fn invariants(&self) -> &[FunctionStats] {
        &self.invariants
    }
}

/// Thread-safe counters for one function.
#[derive(Debug)]
struct FunctionCounters {
    calls: AtomicU64,
    successful: AtomicU64,
    reverts: AtomicU64,
    wall_total_ns: AtomicU64,
    wall_min_ns: AtomicU64,
    wall_max_ns: AtomicU64,
    rpc_hits: AtomicU64,
    rpc_misses: AtomicU64,
    rpc_wait_ns: AtomicU64,
    revert_counts: Mutex<HashMap<RevertKey, u64>>,
}

impl FunctionCounters {
    /// Create zeroed counters with an empty minimum wall time.
    fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            successful: AtomicU64::new(0),
            reverts: AtomicU64::new(0),
            wall_total_ns: AtomicU64::new(0),
            wall_min_ns: AtomicU64::new(u64::MAX),
            wall_max_ns: AtomicU64::new(0),
            rpc_hits: AtomicU64::new(0),
            rpc_misses: AtomicU64::new(0),
            rpc_wait_ns: AtomicU64::new(0),
            revert_counts: Mutex::new(HashMap::new()),
        }
    }

    /// Record one transaction result.
    fn record(&self, result: &TransactionResult) {
        // 1. Count the call outcome.
        self.calls.fetch_add(1, Ordering::Relaxed);
        if result.success {
            self.successful.fetch_add(1, Ordering::Relaxed);
        } else {
            self.reverts.fetch_add(1, Ordering::Relaxed);
        }

        // 2. Accumulate wall time and RPC usage.
        let elapsed_ns = u64::try_from(result.elapsed.as_nanos()).unwrap_or(u64::MAX);
        self.wall_total_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
        self.wall_min_ns.fetch_min(elapsed_ns, Ordering::Relaxed);
        self.wall_max_ns.fetch_max(elapsed_ns, Ordering::Relaxed);
        self.rpc_hits.fetch_add(result.rpc.hits, Ordering::Relaxed);
        self.rpc_misses
            .fetch_add(result.rpc.misses, Ordering::Relaxed);
        let wait_ns = u64::try_from(result.rpc.wait.as_nanos()).unwrap_or(u64::MAX);
        self.rpc_wait_ns.fetch_add(wait_ns, Ordering::Relaxed);

        // 3. Group the revert by its kind and data.
        if let Some(key) = classify(result) {
            *self.revert_counts.lock().entry(key).or_insert(0) += 1;
        }
    }

    /// Snapshot the counters for the given function.
    fn snapshot(&self, function: &Function) -> FunctionStats {
        // 1. Load the call, time, and RPC counters.
        let calls = self.calls.load(Ordering::Relaxed);
        let total_ns = self.wall_total_ns.load(Ordering::Relaxed);
        let min_ns = self.wall_min_ns.load(Ordering::Relaxed);

        // 2. Group the reverts by kind and data, most frequent first.
        let mut reverts: Vec<RevertSummary> = self
            .revert_counts
            .lock()
            .iter()
            .map(|(key, count)| RevertSummary::from_key(key, *count))
            .collect();
        reverts.sort_by(|left, right| {
            right
                .count()
                .cmp(&left.count())
                .then_with(|| left.kind().cmp(right.kind()))
                .then_with(|| left.selector().cmp(&right.selector()))
                .then_with(|| left.name().cmp(&right.name()))
                .then_with(|| left.message().cmp(&right.message()))
                .then_with(|| left.code().cmp(&right.code()))
        });

        // 3. Build the entry with zeroed timings when nothing was recorded.
        FunctionStats {
            name: function.name.clone(),
            selector: selector_hex(function),
            calls,
            successful_calls: self.successful.load(Ordering::Relaxed),
            revert_calls: self.reverts.load(Ordering::Relaxed),
            wall_time_ns: WallTime::new(
                if calls == 0 { 0 } else { min_ns },
                self.wall_max_ns.load(Ordering::Relaxed),
                total_ns.checked_div(calls).unwrap_or(0),
            ),
            rpc: RpcSummary::new()
                .with_hits(self.rpc_hits.load(Ordering::Relaxed))
                .with_misses(self.rpc_misses.load(Ordering::Relaxed))
                .with_wait_ns(self.rpc_wait_ns.load(Ordering::Relaxed)),
            reverts,
        }
    }
}

/// Shared per-function statistics collected across fuzzer threads.
#[derive(Debug, Clone)]
pub struct SharedStats {
    inner: Arc<Inner>,
}

/// The guarded statistics state.
#[derive(Debug)]
struct Inner {
    handlers: Vec<FunctionCounters>,
    invariants: Vec<FunctionCounters>,
}

impl SharedStats {
    /// Create statistics for the given number of handlers and invariants.
    pub fn new(handler_count: usize, invariant_count: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                handlers: (0..handler_count)
                    .map(|_| FunctionCounters::new())
                    .collect(),
                invariants: (0..invariant_count)
                    .map(|_| FunctionCounters::new())
                    .collect(),
            }),
        }
    }

    /// Record one handler call result at the given handler index.
    pub fn record_handler(&self, index: usize, result: &TransactionResult) {
        if let Some(counters) = self.inner.handlers.get(index) {
            counters.record(result);
        }
    }

    /// Record one invariant check result at the given invariant index.
    pub fn record_invariant(&self, index: usize, result: &TransactionResult) {
        if let Some(counters) = self.inner.invariants.get(index) {
            counters.record(result);
        }
    }

    /// Snapshot per-handler statistics in harness order.
    pub fn handler_stats(&self, handlers: &[Function]) -> Vec<FunctionStats> {
        snapshot(&self.inner.handlers, handlers)
    }

    /// Snapshot per-invariant statistics in harness order.
    pub fn invariant_stats(&self, invariants: &[Function]) -> Vec<FunctionStats> {
        snapshot(&self.inner.invariants, invariants)
    }
}

/// Snapshot counters for the given functions in order.
fn snapshot(counters: &[FunctionCounters], functions: &[Function]) -> Vec<FunctionStats> {
    counters
        .iter()
        .zip(functions.iter())
        .map(|(counter, function)| counter.snapshot(function))
        .collect()
}

/// The selector of a function as `0x` prefixed hex.
fn selector_hex(function: &Function) -> String {
    format!("0x{}", hex::encode(function.selector().as_slice()))
}

/// Classify a failed call by its decoded revert.
///
/// Returns `None` for successful calls. Each key carries only its kind data:
///
/// - `BrokenInvariantError` with its selector and `{id}: {description}`
/// - `Error` with its selector and the decoded string
/// - `Panic` with its selector and the code as hex
/// - `CustomError` with the selector, the name resolves later
/// - `EmptyRevert` and `Halt` without data
/// - `UnknownRevert` with the raw output as `0x` prefixed hex
fn classify(result: &TransactionResult) -> Option<RevertKey> {
    // 1. Skip successful calls, they carry no revert.
    if result.success {
        return None;
    }

    // 2. Classify missing output as a halt and empty output as an empty revert.
    let Some(output) = result.output.as_ref() else {
        return Some(RevertKey::Halt);
    };
    if output.is_empty() {
        return Some(RevertKey::EmptyRevert);
    }

    // 3. Decode the explicit broken invariant report.
    if let Some(broken) = BrokenInvariantError::abi_decode(output).ok()
        && !broken.id.is_empty()
    {
        let message = if broken.description.is_empty() {
            broken.id
        } else {
            format!("{}: {}", broken.id, broken.description)
        };
        return Some(RevertKey::BrokenInvariantError {
            selector: format!("0x{}", hex::encode(BrokenInvariantError::SELECTOR)),
            message,
        });
    }

    // 4. Decode the standard Error and Panic reverts.
    if let Ok(error) = Error::abi_decode(output) {
        return Some(RevertKey::Error {
            selector: format!("0x{}", hex::encode(Error::SELECTOR)),
            message: error.message,
        });
    }
    if let Ok(panic) = Panic::abi_decode(output) {
        return Some(RevertKey::Panic {
            selector: format!("0x{}", hex::encode(Panic::SELECTOR)),
            code: format!("{:#x}", panic.code),
        });
    }

    // 5. Fall back to the custom error selector.
    //    The name resolves later from the compilation output.
    if output.len() >= 4 {
        return Some(RevertKey::CustomError {
            selector: format!("0x{}", hex::encode(&output[..4])),
        });
    }
    Some(RevertKey::UnknownRevert {
        message: format!("0x{}", hex::encode(output)),
    })
}

/// Writes fuzzing statistics to `{root}/.ripfuzz/stats`.
///
/// Each report is saved as `{unix-timestamp}-{id}.json` and the absolute path
/// is returned so logs and errors can point at the file.
///
/// ```rust,no_run
/// use ripfuzz::tester::StatsWriter;
///
/// # let stats: ripfuzz::tester::Stats = todo!();
/// let path = StatsWriter::new()
///     .with_root(std::path::Path::new("."))
///     .with_stats(stats)
///     .write()
///     .unwrap();
/// println!("statistics: {}", path.display());
/// ```
#[derive(Debug, Clone, Default)]
pub struct StatsWriter {
    root: Option<PathBuf>,
    stats: Option<Stats>,
    run_id: Option<RunId>,
}

impl StatsWriter {
    /// Create an empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the project root the statistics are saved under.
    pub fn with_root(mut self, root: &Path) -> Self {
        self.root = Some(root.to_path_buf());
        self
    }

    /// Set the statistics report to save.
    pub fn with_stats(mut self, stats: Stats) -> Self {
        self.stats = Some(stats);
        self
    }

    /// Name the report after the run so artifacts of one campaign share
    /// their filename stem. Without it the report gets a timestamped id.
    pub fn with_run_id(mut self, run_id: &RunId) -> Self {
        self.run_id = Some(run_id.clone());
        self
    }

    /// Serialize and save the statistics report, returning its absolute path.
    pub fn write(&self) -> Result<PathBuf> {
        // 1. Require the writer context.
        let root = self
            .root
            .as_ref()
            .context("root not set, call StatsWriter::new().with_root(..)")?;
        let stats = self
            .stats
            .as_ref()
            .context("stats not set, call StatsWriter::new().with_stats(..)")?;

        // 2. Require the report metadata.
        stats
            .metadata()
            .context("metadata not set, call Stats::new().with_metadata(..)")?;

        // 3. Serialize the report and ensure the stats directory exists.
        let report =
            serde_json::to_string_pretty(stats).context("failed to serialize statistics")?;
        let stats_dir = root.join(".ripfuzz").join("stats");
        fs::create_dir_all(&stats_dir)?;

        // 4. Write the report file and return its absolute path.
        let stats_file = stats_dir.join(self.file_name());
        fs::write(&stats_file, report)
            .with_context(|| format!("failed to write {}", stats_file.display()))?;
        Ok(absolute(stats_file)?)
    }

    /// Statistics file name for this writer, without the parent directory.
    fn file_name(&self) -> String {
        // 1. Share the run stem when the command minted one.
        if let Some(run) = &self.run_id {
            return format!("{run}.json");
        }

        // 2. Fall back to a timestamped id for standalone use.
        let timestamp = jiff::Timestamp::now().as_second();
        format!("{timestamp}-{}.json", stats_id())
    }
}

/// Short unique id for a statistics file name.
fn stats_id() -> String {
    let uuid: String = uuid::Uuid::new_v4().into();
    uuid.split('-').next().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use alloy_primitives::U256;
    use alloy_sol_types::SolValue;
    use revm::primitives::Bytes;

    use super::*;
    use crate::evm::RpcStats;

    fn function(signature: &str) -> Function {
        Function::parse(signature).unwrap()
    }

    fn success(elapsed_ns: u64) -> TransactionResult {
        TransactionResult {
            success: true,
            elapsed: Duration::from_nanos(elapsed_ns),
            ..Default::default()
        }
    }

    fn revert(output: Vec<u8>, elapsed_ns: u64, rpc: RpcStats) -> TransactionResult {
        TransactionResult {
            success: false,
            output: Some(Bytes::from(output)),
            elapsed: Duration::from_nanos(elapsed_ns),
            rpc,
            ..Default::default()
        }
    }

    fn halt() -> TransactionResult {
        TransactionResult {
            success: false,
            output: None,
            ..Default::default()
        }
    }

    fn error_output(message: &str) -> Vec<u8> {
        let mut output = Error::SELECTOR.to_vec();
        output.extend((message.to_owned(),).abi_encode_params());
        output
    }

    fn panic_output(code: u64) -> Vec<u8> {
        let mut output = Panic::SELECTOR.to_vec();
        output.extend((U256::from(code),).abi_encode_params());
        output
    }

    fn broken_output(id: &str, description: &str) -> Vec<u8> {
        let mut output = BrokenInvariantError::SELECTOR.to_vec();
        output.extend((id.to_owned(), description.to_owned()).abi_encode_params());
        output
    }

    #[test]
    fn records_handler_success_and_revert_counts() {
        let stats = SharedStats::new(1, 0);
        let handlers = [function("deposit(uint256)")];
        stats.record_handler(0, &success(10));
        stats.record_handler(0, &revert(error_output("nope"), 20, RpcStats::default()));

        let entries = stats.handler_stats(&handlers);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name(), "deposit");
        assert_eq!(entries[0].calls(), 2);
        assert_eq!(entries[0].successful_calls(), 1);
        assert_eq!(entries[0].revert_calls(), 1);
    }

    #[test]
    fn groups_reverts_by_decoded_message() {
        let stats = SharedStats::new(1, 0);
        let handlers = [function("withdraw(uint256)")];
        stats.record_handler(0, &revert(error_output("low"), 10, RpcStats::default()));
        stats.record_handler(0, &revert(error_output("low"), 10, RpcStats::default()));
        stats.record_handler(0, &revert(error_output("high"), 10, RpcStats::default()));
        stats.record_handler(0, &revert(panic_output(17), 10, RpcStats::default()));

        let entries = stats.handler_stats(&handlers);
        let error_selector = format!("0x{}", hex::encode(Error::SELECTOR));
        let panic_selector = format!("0x{}", hex::encode(Panic::SELECTOR));

        assert_eq!(
            entries[0].reverts(),
            &[
                RevertSummary::Error {
                    selector: error_selector.clone(),
                    message: String::from("low"),
                    count: 2,
                },
                RevertSummary::Error {
                    selector: error_selector,
                    message: String::from("high"),
                    count: 1,
                },
                RevertSummary::Panic {
                    selector: panic_selector,
                    code: String::from("0x11"),
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn tracks_wall_time_min_max_avg_and_rpc() {
        let stats = SharedStats::new(1, 0);
        let handlers = [function("deposit(uint256)")];
        let rpc = RpcStats {
            hits: 3,
            misses: 1,
            wait: Duration::from_nanos(40),
        };
        stats.record_handler(0, &success(10));
        stats.record_handler(0, &revert(error_output("nope"), 30, rpc));

        let entries = stats.handler_stats(&handlers);

        assert_eq!(entries[0].wall_time_ns(), WallTime::new(10, 30, 20));
        assert_eq!(
            entries[0].rpc(),
            RpcSummary::new()
                .with_hits(3)
                .with_misses(1)
                .with_wait_ns(40)
        );
    }

    #[test]
    fn keeps_handler_and_invariant_stats_separate() {
        let stats = SharedStats::new(1, 1);
        let handlers = [function("deposit(uint256)")];
        let invariants = [function("invariant_ok()")];
        stats.record_handler(0, &success(10));
        stats.record_invariant(
            0,
            &revert(broken_output("ID-1", "bad"), 10, RpcStats::default()),
        );

        let handler_entries = stats.handler_stats(&handlers);
        let invariant_entries = stats.invariant_stats(&invariants);

        assert_eq!(handler_entries[0].calls(), 1);
        assert_eq!(handler_entries[0].revert_calls(), 0);
        assert_eq!(invariant_entries[0].calls(), 1);
        assert_eq!(invariant_entries[0].revert_calls(), 1);
        let broken_selector = format!("0x{}", hex::encode(BrokenInvariantError::SELECTOR));
        assert_eq!(
            invariant_entries[0].reverts(),
            &[RevertSummary::BrokenInvariantError {
                selector: broken_selector,
                message: String::from("ID-1: bad"),
                count: 1,
            }]
        );
    }

    #[test]
    fn snapshot_maps_names_and_selectors() {
        let stats = SharedStats::new(1, 0);
        let handlers = [function("deposit(uint256)")];
        stats.record_handler(0, &success(10));

        let entries = stats.handler_stats(&handlers);

        assert_eq!(entries[0].name(), "deposit");
        assert_eq!(
            entries[0].selector(),
            format!("0x{}", hex::encode(handlers[0].selector().as_slice()))
        );
    }

    #[test]
    fn classifies_halt_empty_and_custom_reverts() {
        let stats = SharedStats::new(3, 0);
        let handlers = [function("a()"), function("b()"), function("c()")];
        stats.record_handler(0, &halt());
        stats.record_handler(1, &revert(Vec::new(), 10, RpcStats::default()));
        stats.record_handler(
            2,
            &revert(vec![0xde, 0xad, 0xbe, 0xef, 0x01], 10, RpcStats::default()),
        );

        let entries = stats.handler_stats(&handlers);

        assert_eq!(entries[0].reverts(), &[RevertSummary::Halt { count: 1 }]);
        assert_eq!(
            entries[1].reverts(),
            &[RevertSummary::EmptyRevert { count: 1 }]
        );
        assert_eq!(
            entries[2].reverts(),
            &[RevertSummary::CustomError {
                selector: String::from("0xdeadbeef"),
                name: String::new(),
                count: 1,
            }]
        );
    }

    #[test]
    fn resolves_custom_error_names_from_abi() {
        let stats = SharedStats::new(1, 0);
        let handlers = [function("a()")];
        stats.record_handler(
            0,
            &revert(vec![0x77, 0xc5, 0x22, 0xea, 0x01], 10, RpcStats::default()),
        );
        stats.record_handler(
            0,
            &revert(vec![0xde, 0xad, 0xbe, 0xef, 0x01], 10, RpcStats::default()),
        );

        let mut entries = stats.handler_stats(&handlers);
        let abi = JsonAbi::parse(["error ChainAllocationMismatch(uint256)"]).unwrap();
        let error_resolver = ErrorResolver::new().with_abi(abi);
        error_resolver.resolve(&mut entries);

        assert_eq!(
            entries[0].reverts(),
            &[
                RevertSummary::CustomError {
                    selector: String::from("0x77c522ea"),
                    name: String::from("ChainAllocationMismatch"),
                    count: 1,
                },
                RevertSummary::CustomError {
                    selector: String::from("0xdeadbeef"),
                    name: String::new(),
                    count: 1,
                },
            ]
        );
        assert_eq!(
            error_resolver.name("0x77c522ea"),
            Some("ChainAllocationMismatch")
        );
        assert_eq!(error_resolver.name("0xdeadbeef"), None);
    }

    #[test]
    fn ignores_unknown_handler_and_invariant_indexes() {
        let stats = SharedStats::new(1, 1);
        let handlers = [function("deposit(uint256)")];
        let invariants = [function("invariant_ok()")];
        stats.record_handler(7, &success(10));
        stats.record_invariant(7, &success(10));

        assert_eq!(stats.handler_stats(&handlers)[0].calls(), 0);
        assert_eq!(stats.invariant_stats(&invariants)[0].calls(), 0);
    }

    fn metadata() -> StatsMetadata {
        StatsMetadata {
            harness: String::from("Vault"),
            address: String::from("0x0000000000000000000000000000000000000001"),
            chain_id: 31337,
            seed: 7,
            threads: 2,
            max_runs: 100,
            max_calls: 8,
            timeout_secs: None,
            duration_secs: 1.5,
            total_sequences: 100,
            total_handler_calls: 3,
            total_invariant_checks: 3,
            broken_invariants: 1,
            rpc: RpcSummary::new()
                .with_hits(2)
                .with_misses(1)
                .with_wait_ns(9),
        }
    }

    fn entry() -> FunctionStats {
        FunctionStats {
            name: String::from("deposit"),
            selector: String::from("0x12345678"),
            calls: 3,
            successful_calls: 2,
            revert_calls: 1,
            wall_time_ns: WallTime::new(10, 30, 20),
            rpc: RpcSummary::new(),
            reverts: vec![RevertSummary::Error {
                selector: String::from("0x08c379a0"),
                message: String::from("low"),
                count: 1,
            }],
        }
    }

    #[test]
    fn writer_round_trips_report() {
        let dir = tempfile::tempdir().unwrap();
        let stats = Stats::new()
            .with_metadata(metadata())
            .with_handlers_stats(vec![entry()])
            .with_invariants_stats(Vec::new());

        let path = StatsWriter::new()
            .with_root(dir.path())
            .with_stats(stats.clone())
            .write()
            .unwrap();
        let loaded: Stats = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(loaded, stats);
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "stats");
    }

    #[test]
    fn write_without_root_fails() {
        let stats = Stats::new().with_metadata(metadata());

        let err = StatsWriter::new().with_stats(stats).write().unwrap_err();

        assert_eq!(
            err.to_string(),
            "root not set, call StatsWriter::new().with_root(..)"
        );
    }

    #[test]
    fn write_without_stats_fails() {
        let dir = tempfile::tempdir().unwrap();

        let err = StatsWriter::new()
            .with_root(dir.path())
            .write()
            .unwrap_err();

        assert_eq!(
            err.to_string(),
            "stats not set, call StatsWriter::new().with_stats(..)"
        );
    }

    #[test]
    fn file_name_uses_the_run_stem_when_set() {
        let writer = StatsWriter::new().with_run_id(&RunId::new());

        assert!(writer.file_name().ends_with(".json"));
        assert_eq!(
            writer.file_name(),
            format!("{}.json", writer.run_id.unwrap().stem())
        );
    }

    #[test]
    fn file_name_falls_back_to_a_timestamped_id() {
        let name = StatsWriter::new().file_name();
        let (timestamp, id) = name.strip_suffix(".json").unwrap().split_once('-').unwrap();

        assert!(timestamp.parse::<i64>().is_ok());
        assert_eq!(id.len(), 8);
    }

    #[test]
    fn write_without_metadata_fails() {
        let dir = tempfile::tempdir().unwrap();
        let stats = Stats::new().with_handlers_stats(vec![entry()]);

        let err = StatsWriter::new()
            .with_root(dir.path())
            .with_stats(stats)
            .write()
            .unwrap_err();

        assert_eq!(
            err.to_string(),
            "metadata not set, call Stats::new().with_metadata(..)"
        );
    }
}
