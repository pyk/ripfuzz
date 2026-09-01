//! Failed assertions discovered during fuzzing and the shared, deduplicated
//! collection that tracks them across fuzzer threads.
//!
//! A failed assertion is a Solidity `assert` panic (`Panic(0x01)`) raised by
//! a handler call or by an `invariant_*` call checked after each handler
//! call. Other reverts are not assertions.
//!
//! ```rust
//! use alloy_json_abi::Function;
//! use ripfuzz::max::Sequence;
//! use ripfuzz::tester::Finding;
//!
//! // let function = Function::parse("invariant_total()")?;
//! // let finding = Finding::new(sequence, function, revert_output);
//! ```

use std::collections::HashSet;
use std::sync::Arc;

use alloy_json_abi::Function;
use alloy_sol_types::{Panic, PanicKind, Revert, SolError};
use parking_lot::Mutex;
use revm::primitives::Bytes;

use crate::max::Sequence;

/// One failed assertion: the handler calls that reached it, the function
/// whose `assert` panicked, and the revert output.
///
/// The `trigger` is either a handler (then `sequence` holds the calls before
/// it) or an `invariant_*` function (then `sequence` holds the full prefix,
/// because invariants are appended checks and never part of the sequence).
#[derive(Debug, Clone)]
pub struct Finding {
    sequence: Sequence,
    trigger: Function,
    reason: Bytes,
}

impl Finding {
    /// Create a finding from its sequence, trigger function, and revert
    /// output.
    pub fn new(sequence: Sequence, trigger: Function, reason: Bytes) -> Self {
        Self {
            sequence,
            trigger,
            reason,
        }
    }

    /// The handler calls executed before the assertion failed.
    pub fn sequence(&self) -> &Sequence {
        &self.sequence
    }

    /// The function whose `assert` panicked.
    pub fn trigger(&self) -> &Function {
        &self.trigger
    }

    /// The revert output of the failed assertion.
    pub fn reason(&self) -> &Bytes {
        &self.reason
    }

    /// The human-readable revert reason.
    ///
    /// `Panic(0x01)` renders as `assertion failed`, `Error(string)` renders
    /// as the string, and anything else renders as hex.
    pub fn reason_display(&self) -> String {
        decode_reason(&self.reason)
    }

    /// The key that identifies a distinct failed assertion: the trigger
    /// signature plus the revert output.
    pub fn key(&self) -> String {
        format!("{}|{}", self.trigger.signature(), hex::encode(&self.reason))
    }
}

/// Decode revert output into a human-readable reason.
fn decode_reason(output: &Bytes) -> String {
    if let Ok(panic) = Panic::abi_decode(output) {
        if panic.kind() == Some(PanicKind::Assert) {
            return "assertion failed".to_owned();
        }
        return format!("Panic({:#x})", panic.code.to::<u64>());
    }
    if let Ok(revert) = Revert::abi_decode(output) {
        return revert.reason;
    }
    format!("0x{}", hex::encode(output))
}

/// Shared, deduplicated collection of findings across fuzzer threads.
#[derive(Debug, Clone)]
pub struct SharedFindings {
    inner: Arc<Mutex<Inner>>,
    max: usize,
}

/// The guarded collection state.
#[derive(Debug, Default)]
struct Inner {
    findings: Vec<Finding>,
    keys: HashSet<String>,
}

impl SharedFindings {
    /// Create a collection that holds up to `max` distinct findings.
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

    /// Add a finding when its key is new, returning whether it was added.
    pub fn try_add(&self, finding: &Finding) -> bool {
        let mut inner = self.lock();
        if inner.findings.len() >= self.max {
            return false;
        }
        if !inner.keys.insert(finding.key()) {
            return false;
        }
        inner.findings.push(finding.clone());
        true
    }

    /// Whether the collection reached its capacity.
    pub fn is_full(&self) -> bool {
        self.lock().findings.len() >= self.max
    }

    /// The number of findings collected so far.
    pub fn len(&self) -> usize {
        self.lock().findings.len()
    }

    /// Whether no finding has been collected yet.
    pub fn is_empty(&self) -> bool {
        self.lock().findings.is_empty()
    }

    /// Snapshot all findings in discovery order.
    pub fn findings(&self) -> Vec<Finding> {
        self.lock().findings.clone()
    }
}

#[cfg(test)]
mod tests {
    use alloy_json_abi::Function;

    use super::*;

    fn trigger() -> Function {
        Function::parse("invariant_total()").unwrap()
    }

    /// The Panic(0x01) revert output for a failed `assert`.
    fn assert_output() -> Bytes {
        let mut data = vec![0x4e, 0x48, 0x7b, 0x71];
        data.extend_from_slice(&[0u8; 31]);
        data.push(0x01);
        Bytes::from(data)
    }

    #[test]
    fn reason_display_renders_assertion_failure() {
        let finding = Finding::new(Sequence::empty(), trigger(), assert_output());

        assert_eq!(finding.reason_display(), "assertion failed");
    }

    #[test]
    fn reason_display_renders_revert_string() {
        let revert = Revert::from("balance overflow".to_owned());
        let finding = Finding::new(Sequence::empty(), trigger(), revert.abi_encode().into());

        assert_eq!(finding.reason_display(), "balance overflow");
    }

    #[test]
    fn reason_display_renders_unknown_output_as_hex() {
        let finding = Finding::new(Sequence::empty(), trigger(), Bytes::from([0xde, 0xad]));

        assert_eq!(finding.reason_display(), "0xdead");
    }

    #[test]
    fn try_add_deduplicates_by_trigger_and_reason() {
        let findings = SharedFindings::new(8);
        let first = Finding::new(Sequence::empty(), trigger(), assert_output());
        let same_assert = Finding::new(Sequence::new(vec![]), trigger(), assert_output());

        assert!(findings.try_add(&first));
        assert!(!findings.try_add(&same_assert));
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn try_add_keeps_distinct_assertions() {
        let findings = SharedFindings::new(8);
        let handler = Function::parse("deposit(uint256)").unwrap();
        let invariant_finding = Finding::new(Sequence::empty(), trigger(), assert_output());
        let handler_finding = Finding::new(Sequence::empty(), handler, assert_output());

        assert!(findings.try_add(&invariant_finding));
        assert!(findings.try_add(&handler_finding));
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn try_add_stops_at_capacity() {
        let findings = SharedFindings::new(1);
        let overflow = Function::parse("invariant_other()").unwrap();

        assert!(findings.try_add(&Finding::new(Sequence::empty(), trigger(), assert_output())));
        assert!(!findings.is_empty());
        assert!(findings.is_full());
        assert!(!findings.try_add(&Finding::new(Sequence::empty(), overflow, assert_output())));
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn findings_snapshot_preserves_discovery_order() {
        let findings = SharedFindings::new(8);
        let handler = Function::parse("deposit(uint256)").unwrap();
        findings.try_add(&Finding::new(Sequence::empty(), trigger(), assert_output()));
        findings.try_add(&Finding::new(Sequence::empty(), handler, assert_output()));

        let snapshot = findings.findings();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].trigger().signature(), "invariant_total()");
        assert_eq!(snapshot[1].trigger().signature(), "deposit(uint256)");
    }
}
