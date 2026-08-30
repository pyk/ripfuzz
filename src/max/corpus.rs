//! Corpus of interesting sequences.
//!
//! [`Corpus`] stores sequences that found new coverage or a new best value.
//! Fuzzers draw from it to generate mutations instead of only random
//! sequences, which keeps the search exploring useful state transitions.
//!
//! ```rust
//! use ripfuzz::max::Corpus;
//!
//! // let corpus = Corpus::new();
//! // corpus.add(sequence, value, new_edges);
//! // let base = corpus.random(&mut rng);
//! ```

use std::sync::{Arc, Mutex};

use fastrand::Rng;

use crate::max::{Sequence, Value};

/// Maximum number of entries kept in the corpus.
const MAX_ENTRIES: usize = 256;

/// An interesting sequence with its metadata.
#[derive(Debug, Clone)]
struct Entry {
    sequence: Sequence,
    value: Value,
    new_edges: u64,
}

/// Corpus of interesting sequences shared across fuzzers.
///
/// Cloning is cheap (shares the same inner entries).
#[derive(Debug, Clone)]
pub struct Corpus {
    inner: Arc<Mutex<Vec<Entry>>>,
}

impl Corpus {
    /// Create a new empty corpus.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Lock the entries, recovering from poisoning.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The number of entries in the corpus.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether the corpus has no entries.
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// A random sequence from the corpus.
    pub fn random(&self, rng: &mut Rng) -> Option<Sequence> {
        let entries = self.lock();
        if entries.is_empty() {
            return None;
        }
        let index = rng.usize(..entries.len());
        Some(entries[index].sequence.clone())
    }

    /// Add an interesting sequence.
    ///
    /// When the corpus is full, the entry that brought the fewest new edges
    /// is replaced, and only when the new sequence brings at least as many.
    pub fn add(&self, sequence: Sequence, value: Value, new_edges: u64) {
        let mut entries = self.lock();
        if entries.len() >= MAX_ENTRIES {
            let weakest = entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| (entry.new_edges, entry.value))
                .map(|(index, _)| index);
            let Some(weakest) = weakest else {
                return;
            };
            if new_edges <= entries[weakest].new_edges {
                return;
            }
            entries.remove(weakest);
        }
        entries.push(Entry {
            sequence,
            value,
            new_edges,
        });
    }
}

impl Default for Corpus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloy_json_abi::Function;
    use alloy_primitives::U256;
    use fastrand::Rng;

    use super::*;
    use crate::max::Sequence;

    fn random_sequence() -> Sequence {
        let handlers = [
            Function::parse("a()").unwrap(),
            Function::parse("b()").unwrap(),
        ];
        Sequence::random(&mut Rng::new(), &handlers, 3).unwrap()
    }

    #[test]
    fn random_on_empty_corpus_is_none() {
        let corpus = Corpus::new();
        assert!(corpus.is_empty());
        assert!(corpus.random(&mut Rng::new()).is_none());
    }

    #[test]
    fn add_and_random_return_entries() {
        let corpus = Corpus::new();
        corpus.add(random_sequence(), Value::new(U256::from(1)), 3);
        corpus.add(random_sequence(), Value::new(U256::from(2)), 5);

        assert_eq!(corpus.len(), 2);
        let random = corpus.random(&mut Rng::new()).unwrap();
        assert!(!random.is_empty());
    }

    #[test]
    fn full_corpus_replaces_the_weakest_entry() {
        let corpus = Corpus::new();
        for _ in 0..256 {
            corpus.add(random_sequence(), Value::new(U256::from(1)), 5);
        }
        // A sequence with fewer new edges than the weakest entry is rejected.
        corpus.add(random_sequence(), Value::new(U256::from(9)), 4);
        assert_eq!(corpus.len(), 256);

        // A sequence with more new edges replaces the weakest entry.
        corpus.add(random_sequence(), Value::new(U256::from(9)), 10);
        assert_eq!(corpus.len(), 256);
    }
}
