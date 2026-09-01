//! Broken invariants discovered during fuzzing and the shared, deduplicated
//! collection that tracks them across fuzzer threads.
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
use std::sync::Arc;

use parking_lot::Mutex;

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
