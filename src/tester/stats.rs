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

use alloy_json_abi::Function;
use alloy_sol_types::SolError;
use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::evm::TransactionResult;

alloy_sol_types::sol! {
    error Error(string message);
    error Panic(uint256 code);
    error BrokenInvariantError(string id, string description);
}

/// One grouped revert: the decoded kind and message with its call count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevertSummary {
    kind: String,
    message: String,
    count: u64,
}

impl RevertSummary {
    /// Create a grouped revert entry.
    pub fn new(kind: &str, message: &str, count: u64) -> Self {
        Self {
            kind: kind.to_owned(),
            message: message.to_owned(),
            count,
        }
    }

    /// The revert kind, one of `Error`, `Panic`, `CustomError`,
    /// `BrokenInvariantError`, `EmptyRevert`, `Halt`, or `UnknownRevert`.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The decoded message. Empty for kinds without a payload.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The number of calls that reverted with this kind and message.
    pub fn count(&self) -> u64 {
        self.count
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

    /// Reverts grouped by decoded kind and message, most frequent first.
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
    revert_counts: Mutex<HashMap<(String, String), u64>>,
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

        // 3. Group the revert by its decoded kind and message.
        if let Some((kind, message)) = classify(result) {
            *self
                .revert_counts
                .lock()
                .entry((kind, message))
                .or_insert(0) += 1;
        }
    }

    /// Snapshot the counters for the given function.
    fn snapshot(&self, function: &Function) -> FunctionStats {
        // 1. Load the call, time, and RPC counters.
        let calls = self.calls.load(Ordering::Relaxed);
        let total_ns = self.wall_total_ns.load(Ordering::Relaxed);
        let min_ns = self.wall_min_ns.load(Ordering::Relaxed);

        // 2. Group the reverts by kind and message, most frequent first.
        let mut reverts: Vec<RevertSummary> = self
            .revert_counts
            .lock()
            .iter()
            .map(|((kind, message), count)| RevertSummary::new(kind, message, *count))
            .collect();
        reverts.sort_by(|left, right| {
            right
                .count()
                .cmp(&left.count())
                .then_with(|| left.kind().cmp(right.kind()))
                .then_with(|| left.message().cmp(right.message()))
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
/// Returns `None` for successful calls. The kinds are:
///
/// - `BrokenInvariantError` with `{id}: {description}`
/// - `Error` with the decoded string message
/// - `Panic` with the code as hex
/// - `CustomError` with the selector as `0x` prefixed hex
/// - `EmptyRevert` and `Halt` without a message
/// - `UnknownRevert` with the raw output as `0x` prefixed hex
fn classify(result: &TransactionResult) -> Option<(String, String)> {
    // 1. Skip successful calls, they carry no revert.
    if result.success {
        return None;
    }

    // 2. Classify missing output as a halt and empty output as an empty revert.
    let Some(output) = result.output.as_ref() else {
        return Some((String::from("Halt"), String::new()));
    };
    if output.is_empty() {
        return Some((String::from("EmptyRevert"), String::new()));
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
        return Some((String::from("BrokenInvariantError"), message));
    }

    // 4. Decode the standard Error and Panic reverts.
    if let Ok(error) = Error::abi_decode(output) {
        return Some((String::from("Error"), error.message));
    }
    if let Ok(panic) = Panic::abi_decode(output) {
        return Some((String::from("Panic"), format!("{:#x}", panic.code)));
    }

    // 5. Fall back to the custom error selector.
    if output.len() >= 4 {
        return Some((
            String::from("CustomError"),
            format!("0x{}", hex::encode(&output[..4])),
        ));
    }
    Some((
        String::from("UnknownRevert"),
        format!("0x{}", hex::encode(output)),
    ))
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

        // 4. Write the timestamped report file and return its absolute path.
        let timestamp = jiff::Timestamp::now().as_second();
        let stats_file = stats_dir.join(format!("{timestamp}-{}.json", stats_id()));
        fs::write(&stats_file, report)
            .with_context(|| format!("failed to write {}", stats_file.display()))?;
        Ok(absolute(stats_file)?)
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

        assert_eq!(
            entries[0].reverts(),
            &[
                RevertSummary::new("Error", "low", 2),
                RevertSummary::new("Error", "high", 1),
                RevertSummary::new("Panic", "0x11", 1),
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
        assert_eq!(
            invariant_entries[0].reverts(),
            &[RevertSummary::new("BrokenInvariantError", "ID-1: bad", 1)]
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

        assert_eq!(entries[0].reverts(), &[RevertSummary::new("Halt", "", 1)]);
        assert_eq!(
            entries[1].reverts(),
            &[RevertSummary::new("EmptyRevert", "", 1)]
        );
        assert_eq!(
            entries[2].reverts(),
            &[RevertSummary::new("CustomError", "0xdeadbeef", 1)]
        );
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
            reverts: vec![RevertSummary::new("Error", "low", 1)],
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
