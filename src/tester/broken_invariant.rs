//! Broken invariants discovered during fuzzing, the shared, deduplicated
//! collection that tracks them across fuzzer threads, and the reporter that
//! re-runs a broken invariant and saves its execution trace.
//!
//! A broken invariant is an explicit `rvm.bail` report emitted by a handler
//! call or by an `invariant_*` call checked after each handler call.
//!
//! ```rust,no_run
//! use ripfuzz::tester::{BrokenInvariant, Sequence};
//!
//! # let sequence = Sequence::empty();
//! let broken = BrokenInvariant::new()
//!     .with_calls(sequence)
//!     .with_id("INV-001")
//!     .with_description("total exceeded 100");
//! println!("{}", broken.reason_display());
//! ```

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};
use parking_lot::Mutex;
use revm::primitives::Bytes;
use tracing::info;

use crate::evm::{Chain, Trace, TraceContext, Transaction};
use crate::tester::Sequence;

/// One broken invariant: the calls that reproduce it, ending with the
/// bail-emitting call, and the explicit metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrokenInvariant {
    sequence: Sequence,
    id: String,
    description: String,
}

impl BrokenInvariant {
    /// Create an empty broken invariant.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the broken invariant id, the dedup key of the collection.
    pub fn with_id(mut self, id: &str) -> Self {
        self.id = id.to_owned();
        self
    }

    /// Set the broken invariant description.
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = description.to_owned();
        self
    }

    /// Set the calls that reproduce the broken invariant, ending with the
    /// bail-emitting call.
    pub fn with_calls(mut self, sequence: Sequence) -> Self {
        self.sequence = sequence;
        self
    }

    /// The calls that reproduce the broken invariant, ending with the
    /// bail-emitting call.
    pub fn sequence(&self) -> &Sequence {
        &self.sequence
    }

    /// The broken invariant id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The broken invariant description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The human-readable reason, preferring description over id.
    pub fn reason_display(&self) -> String {
        if !self.description.is_empty() {
            return self.description.clone();
        }
        self.id.clone()
    }

    /// The key that identifies a distinct broken invariant: the `id` alone.
    pub fn key(&self) -> String {
        self.id.clone()
    }
}

/// Shared, deduplicated collection of broken invariants across fuzzer
/// threads.
#[derive(Debug, Clone)]
pub struct SharedBrokenInvariants {
    inner: Arc<Mutex<Inner>>,
    max: usize,
}

/// The guarded collection state.
#[derive(Debug, Default)]
struct Inner {
    broken_invariants: Vec<BrokenInvariant>,
    keys: HashSet<String>,
}

impl SharedBrokenInvariants {
    /// Create a collection that holds up to `max` distinct broken invariants.
    pub fn new(max: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            max,
        }
    }

    /// Lock the collection state.
    fn lock(&self) -> parking_lot::MutexGuard<'_, Inner> {
        self.inner.lock()
    }

    /// Add a broken invariant when its id is new, returning whether it was
    /// added.
    pub fn try_add(&self, broken: &BrokenInvariant) -> bool {
        let mut inner = self.lock();
        if inner.broken_invariants.len() >= self.max {
            return false;
        }
        if !inner.keys.insert(broken.key()) {
            return false;
        }
        inner.broken_invariants.push(broken.clone());
        true
    }

    /// Whether the collection reached its capacity.
    pub fn is_full(&self) -> bool {
        self.lock().broken_invariants.len() >= self.max
    }

    /// The number of broken invariants collected so far.
    pub fn len(&self) -> usize {
        self.lock().broken_invariants.len()
    }

    /// Whether no broken invariant has been collected yet.
    pub fn is_empty(&self) -> bool {
        self.lock().broken_invariants.is_empty()
    }

    /// Snapshot all broken invariants in discovery order.
    pub fn all(&self) -> Vec<BrokenInvariant> {
        self.lock().broken_invariants.clone()
    }
}

/// Re-runs a broken invariant on a traced chain clone and saves its
/// execution trace under `{root}/.ripfuzz/traces`.
///
/// The re-run transaction batch is the broken invariant's sequence, whose
/// last call is the bail-emitting trigger, plus the optional summary call.
///
/// ```rust,no_run
/// use ripfuzz::tester::{BrokenInvariant, BrokenInvariantReporter};
/// use ripfuzz::{Chain, ChainConfig, TraceContext};
///
/// # let chain = Chain::empty(ChainConfig::default());
/// # let broken = BrokenInvariant::new().with_id("INV-001");
/// let trace_context = TraceContext::new();
/// let reporter = BrokenInvariantReporter::new(std::path::Path::new("."))
///     .with_chain(&chain)
///     .with_trace_context(&trace_context)
///     .with_address(chain.deployer());
/// reporter.report(&broken).unwrap();
/// ```
#[derive(Debug)]
pub struct BrokenInvariantReporter {
    root: PathBuf,
    chain: Option<Chain>,
    trace_context: Option<TraceContext>,
    address: Option<Address>,
    summary: Option<Function>,
}

impl BrokenInvariantReporter {
    /// Create a reporter that saves traces under the project root.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            chain: None,
            trace_context: None,
            address: None,
            summary: None,
        }
    }

    /// Set the chain the broken invariant is re-run on.
    pub fn with_chain(mut self, chain: &Chain) -> Self {
        self.chain = Some(chain.clone());
        self
    }

    /// Set the trace context used to format the saved trace.
    pub fn with_trace_context(mut self, trace_context: &TraceContext) -> Self {
        self.trace_context = Some(trace_context.clone());
        self
    }

    /// Set the address the sequence calls are sent to.
    pub fn with_address(mut self, address: Address) -> Self {
        self.address = Some(address);
        self
    }

    /// Set the optional summary call appended after the sequence.
    pub fn with_summary(mut self, summary: Option<&Function>) -> Self {
        self.summary = summary.cloned();
        self
    }

    /// Re-run the broken invariant on a traced chain clone and save its
    /// execution trace.
    pub fn report(&self, broken: &BrokenInvariant) -> Result<()> {
        // 1. Require the execution context.
        let chain = self
            .chain
            .as_ref()
            .context("chain not set, call BrokenInvariantReporter::new().with_chain(..)")?;
        let trace_context = self.trace_context.as_ref().context(
            "trace context not set, call BrokenInvariantReporter::new().with_trace_context(..)",
        )?;
        let address = self
            .address
            .context("address not set, call BrokenInvariantReporter::new().with_address(..)")?;

        // 2. Build the traced re-run with the sequence and the summary.
        let deployer = chain.deployer();
        let mut rerun_chain = chain.clone();
        rerun_chain.set_trace(true);
        let mut transactions: Vec<Transaction> = broken.sequence().transactions(address, deployer);
        if let Some(summary) = &self.summary {
            transactions.push(
                Transaction::new(address)
                    .calldata(Bytes::from(summary.selector().as_slice().to_vec())),
            );
        }

        // 3. Execute the re-run, the bail-emitting trigger is expected and
        //    does not invalidate the logs of the calls before it.
        let output = rerun_chain.exec(&transactions)?;

        // 4. Save the execution trace for offline analysis.
        let trace = output
            .trace
            .context("broken invariant re-run trace missing")?;
        let trace_file = self.save_trace(trace_context, &trace)?;
        info!(id = %broken.id(), trace = %trace_file.display(), "broken invariant saved");
        Ok(())
    }

    /// Save an execution trace under `{root}/.ripfuzz/traces` and return its
    /// path relative to the root for logging.
    fn save_trace(&self, trace_context: &TraceContext, trace: &Trace) -> Result<PathBuf> {
        // 1. Write the execution trace to a timestamped trace file.
        let trace_dir = self.root.join(".ripfuzz").join("traces");
        fs::create_dir_all(&trace_dir)?;
        let timestamp = jiff::Timestamp::now().as_second();
        let trace_file = trace_dir.join(format!("{timestamp}-{}.log", trace_id()));
        let trace = trace.display_with(trace_context).to_string();
        fs::write(&trace_file, trace)
            .with_context(|| format!("failed to write {}", trace_file.display()))?;

        // 2. Return the path relative to the root so logs stay portable.
        let relative = trace_file.strip_prefix(&self.root).unwrap_or(&trace_file);
        Ok(relative.to_path_buf())
    }
}

/// Short unique id for a trace file name.
fn trace_id() -> String {
    let uuid: String = uuid::Uuid::new_v4().into();
    uuid.split('-').next().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn broken_with_id(id: &str) -> BrokenInvariant {
        BrokenInvariant::new()
            .with_calls(Sequence::empty())
            .with_id(id)
            .with_description("desc")
    }

    #[test]
    fn reason_display_prefers_description() {
        let broken = BrokenInvariant::new()
            .with_id("ID-001")
            .with_description("my desc");

        assert_eq!(broken.reason_display(), "my desc");
    }

    #[test]
    fn reason_display_falls_back_to_id() {
        let broken = BrokenInvariant::new().with_id("ID-001");

        assert_eq!(broken.reason_display(), "ID-001");
    }

    #[test]
    fn try_add_deduplicates_by_id() {
        let broken_invariants = SharedBrokenInvariants::new(8);
        let first = broken_with_id("ID-001");
        let same = BrokenInvariant::new()
            .with_id("ID-001")
            .with_description("other");

        assert!(broken_invariants.try_add(&first));
        assert!(!broken_invariants.try_add(&same));
        assert_eq!(broken_invariants.len(), 1);
    }

    #[test]
    fn try_add_keeps_distinct_ids() {
        let broken_invariants = SharedBrokenInvariants::new(8);
        let first = broken_with_id("ID-001");
        let second = BrokenInvariant::new()
            .with_id("ID-002")
            .with_description("desc");

        assert!(broken_invariants.try_add(&first));
        assert!(broken_invariants.try_add(&second));
        assert_eq!(broken_invariants.len(), 2);
    }

    #[test]
    fn try_add_stops_at_capacity() {
        let broken_invariants = SharedBrokenInvariants::new(1);
        let overflow = BrokenInvariant::new().with_id("ID-002");

        assert!(broken_invariants.try_add(&broken_with_id("ID-001")));
        assert!(!broken_invariants.is_empty());
        assert!(broken_invariants.is_full());
        assert!(!broken_invariants.try_add(&overflow));
        assert_eq!(broken_invariants.len(), 1);
    }

    #[test]
    fn all_preserves_discovery_order() {
        let broken_invariants = SharedBrokenInvariants::new(8);
        broken_invariants.try_add(&broken_with_id("ID-001"));
        broken_invariants.try_add(&BrokenInvariant::new().with_id("ID-002"));

        let snapshot = broken_invariants.all();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].id(), "ID-001");
        assert_eq!(snapshot[1].id(), "ID-002");
    }

    #[test]
    fn key_is_id() {
        let broken = broken_with_id("MY-ID");
        assert_eq!(broken.key(), "MY-ID");
    }
}
