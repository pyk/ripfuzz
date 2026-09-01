//! Findings discovered during fuzzing and the shared, deduplicated
//! collection that tracks them across fuzzer threads.
//!
//! A finding is an explicit `rvm.finding` cheatcode report emitted by a
//! handler call or by an `invariant_*` call checked after each handler call.
//!
//! ```rust
//! use alloy_json_abi::Function;
//! use ripfuzz::tester::{Finding, Sequence, Severity};
//!
//! // let function = Function::parse("invariant_total()")?;
//! // let finding = Finding::new(sequence, function, "FIND-001", Severity::High, "title", "desc");
//! ```

use std::collections::HashSet;
use std::sync::Arc;

use alloy_json_abi::Function;
use parking_lot::Mutex;

use crate::evm::ReportedFinding;
use crate::evm::Severity;
use crate::tester::Sequence;

/// One finding: the handler calls that reached it and the explicit metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    sequence: Sequence,
    trigger: Function,
    id: String,
    severity: Severity,
    title: String,
    description: String,
}

impl Finding {
    /// Create a finding from its sequence, trigger function, and explicit
    /// metadata.
    pub fn new(
        sequence: Sequence,
        trigger: Function,
        id: &str,
        severity: Severity,
        title: &str,
        description: &str,
    ) -> Self {
        Self {
            sequence,
            trigger,
            id: id.to_owned(),
            severity,
            title: title.to_owned(),
            description: description.to_owned(),
        }
    }

    /// Create a finding from a [`ReportedFinding`] emitted via `rvm.finding`.
    pub fn new_explicit(sequence: Sequence, trigger: Function, finding: ReportedFinding) -> Self {
        Self {
            sequence,
            trigger,
            id: finding.id,
            severity: finding.severity,
            title: finding.title,
            description: finding.description,
        }
    }

    /// Create a finding from raw fields.
    pub fn new_explicit_with_meta(
        sequence: Sequence,
        trigger: Function,
        id: &str,
        severity: Severity,
        title: &str,
        description: &str,
    ) -> Self {
        Self {
            sequence,
            trigger,
            id: id.to_owned(),
            severity,
            title: title.to_owned(),
            description: description.to_owned(),
        }
    }

    /// The handler calls executed before the finding was emitted.
    pub fn sequence(&self) -> &Sequence {
        &self.sequence
    }

    /// The function that emitted the finding.
    pub fn trigger(&self) -> &Function {
        &self.trigger
    }

    /// The finding id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The finding severity.
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// The finding title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The finding description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The human-readable reason, preferring title over description and id.
    pub fn reason_display(&self) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        if !self.description.is_empty() {
            return self.description.clone();
        }
        self.id.clone()
    }

    /// The key that identifies a distinct finding: the `id` alone.
    pub fn key(&self) -> String {
        self.id.clone()
    }
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

    /// Add a finding when its id is new, returning whether it was added.
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

    fn finding_with_id(id: &str) -> Finding {
        Finding::new(
            Sequence::empty(),
            trigger(),
            id,
            Severity::Medium,
            "title",
            "desc",
        )
    }

    #[test]
    fn reason_display_prefers_title() {
        let finding = Finding::new(
            Sequence::empty(),
            trigger(),
            "ID-001",
            Severity::High,
            "my title",
            "my desc",
        );

        assert_eq!(finding.reason_display(), "my title");
    }

    #[test]
    fn reason_display_falls_back_to_description() {
        let finding = Finding::new(
            Sequence::empty(),
            trigger(),
            "ID-001",
            Severity::High,
            "",
            "my desc",
        );

        assert_eq!(finding.reason_display(), "my desc");
    }

    #[test]
    fn reason_display_falls_back_to_id() {
        let finding = Finding::new(
            Sequence::empty(),
            trigger(),
            "ID-001",
            Severity::High,
            "",
            "",
        );

        assert_eq!(finding.reason_display(), "ID-001");
    }

    #[test]
    fn try_add_deduplicates_by_id() {
        let findings = SharedFindings::new(8);
        let first = finding_with_id("ID-001");
        let same = Finding::new(
            Sequence::new(vec![]),
            trigger(),
            "ID-001",
            Severity::High,
            "other",
            "other",
        );

        assert!(findings.try_add(&first));
        assert!(!findings.try_add(&same));
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn try_add_keeps_distinct_ids() {
        let findings = SharedFindings::new(8);
        let first = finding_with_id("ID-001");
        let second = Finding::new(
            Sequence::empty(),
            trigger(),
            "ID-002",
            Severity::Low,
            "title",
            "desc",
        );

        assert!(findings.try_add(&first));
        assert!(findings.try_add(&second));
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn try_add_stops_at_capacity() {
        let findings = SharedFindings::new(1);
        let overflow = Finding::new(
            Sequence::empty(),
            Function::parse("invariant_other()").unwrap(),
            "ID-002",
            Severity::Medium,
            "",
            "",
        );

        assert!(findings.try_add(&finding_with_id("ID-001")));
        assert!(!findings.is_empty());
        assert!(findings.is_full());
        assert!(!findings.try_add(&overflow));
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn findings_snapshot_preserves_discovery_order() {
        let findings = SharedFindings::new(8);
        let handler = Function::parse("deposit(uint256)").unwrap();
        findings.try_add(&finding_with_id("ID-001"));
        findings.try_add(&Finding::new(
            Sequence::empty(),
            handler,
            "ID-002",
            Severity::Medium,
            "",
            "",
        ));

        let snapshot = findings.findings();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].id(), "ID-001");
        assert_eq!(snapshot[1].id(), "ID-002");
    }

    #[test]
    fn key_is_id() {
        let finding = finding_with_id("MY-ID");
        assert_eq!(finding.key(), "MY-ID");
    }
}
