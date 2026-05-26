//! Thread-safe corpus shared across parallel fuzzer threads.
//!
//! ## Separation of concerns
//!
//! `SharedCorpus` is responsible for:
//! - Loading and validating corpus from disk.
//! - Defining [`CorpusItem`] which is convertible to/from
//!   [`evm::chain::ExecInput`](crate::evm::chain::ExecInput).
//! - Serializing corpus items as compact JSON.
//! - Providing [`take`](SharedCorpus::take) to return a randomly selected
//!   corpus item (mutated when sourced from the existing pool) for a fuzzer
//!   thread.
//! - Providing [`add`](SharedCorpus::add) to add interesting sequences to
//!   the collection.
//!
//! [`Fuzzer`](crate::fuzzer::factory::Fuzzer) is responsible for:
//! - Using [`take`](SharedCorpus::take) to obtain the next input to execute.
//! - Using [`add`](SharedCorpus::add) to store interesting sequences
//!   discovered during execution.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

pub use call::{Call, CallMeta};
pub use corpus::{Corpus, CorpusItem};

use crate::evm::coverage::map::LocalCoverage;
use crate::fuzzer::config::Config;
use crate::fuzzer::mutators::Mutator;
use crate::target::Contract;

pub mod call;
#[allow(clippy::module_inception)]
pub mod corpus;

/// Statistics produced by loading a corpus from disk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CorpusStats {
    pub total_count: usize,
    pub parse_failed_count: usize,
    pub invalid_call_count: usize,
    pub valid_count: usize,
}

/// Runtime statistics for the shared corpus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub item_count: usize,
    pub failure_count: usize,
    pub coverage_hits: usize,
}

/// Inner state of [`SharedCorpus`].
///
/// All mutable data is protected by the `corpus` lock; `mutators` is
/// read-only after construction so it can be accessed lock-free.
pub struct SharedCorpusInner {
    pub corpus: std::sync::RwLock<Corpus>,
    pub contract: Arc<Contract>,
    pub selectors: Vec<[u8; 4]>,
    pub mutators: Vec<Box<dyn Mutator>>,
}

impl std::fmt::Debug for SharedCorpusInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedCorpusInner")
            .field("corpus", &self.corpus)
            .field("contract", &self.contract)
            .field("selectors", &self.selectors)
            .field(
                "mutators",
                &format_args!("[{} mutators]", self.mutators.len()),
            )
            .finish()
    }
}

/// Thread-safe corpus shared across parallel fuzzer threads.
///
/// Cloning is cheap (shares the same inner state).
#[derive(Debug, Clone)]
pub struct SharedCorpus {
    inner: Arc<SharedCorpusInner>,
}

impl SharedCorpus {
    /// Create an empty corpus backed by `dir` for the given target contract.
    ///
    /// No disk I/O is performed until [`Self::load`] is called.
    pub fn new(dir: impl AsRef<Path>, contract: Contract, config: Config) -> Self {
        let selectors: Vec<[u8; 4]> = contract
            .target_functions
            .iter()
            .map(|f| f.selector().into())
            .collect();

        let mut corpus = Corpus::new();
        corpus.set_storage_dir(dir);

        let inner = Arc::new_cyclic(|weak| {
            let mutators: Vec<Box<dyn Mutator>> = vec![
                Box::new(crate::fuzzer::mutators::SequenceSwapMutator),
                Box::new(crate::fuzzer::mutators::SequenceInsertMutator::new(
                    selectors.clone(),
                    config.max_block_number_delay,
                    config.max_block_timestamp_delay,
                )),
                Box::new(crate::fuzzer::mutators::SequenceDeleteMutator),
                Box::new(crate::fuzzer::mutators::SequenceSpliceMutator::new(
                    weak.clone(),
                )),
                Box::new(crate::fuzzer::mutators::SequenceInterleaveMutator::new(
                    weak.clone(),
                )),
                Box::new(crate::fuzzer::mutators::SequenceHeadMutator::new(
                    weak.clone(),
                )),
                Box::new(crate::fuzzer::mutators::SequenceTailMutator::new(
                    weak.clone(),
                )),
                Box::new(crate::fuzzer::mutators::SequenceArgMutator::new(
                    contract.abi.clone(),
                )),
                Box::new(crate::fuzzer::mutators::SequenceDelayMutator::new(
                    config.max_block_number_delay,
                    config.max_block_timestamp_delay,
                )),
            ];

            SharedCorpusInner {
                corpus: std::sync::RwLock::new(corpus),
                contract: Arc::new(contract),
                selectors,
                mutators,
            }
        });

        Self { inner }
    }

    /// Load corpus items from the storage directory and validate them
    /// against the target contract ABI.
    ///
    /// Valid items are added to the pending queue. Invalid or unparsable
    /// items are counted in the returned [`CorpusStats`] but are not stored.
    pub fn load(&self) -> Result<CorpusStats> {
        let mut total_count = 0usize;
        let mut parse_failed_count = 0usize;
        let mut invalid_call_count = 0usize;
        let mut valid_count = 0usize;

        let dir = {
            let guard = self
                .inner
                .corpus
                .read()
                .map_err(|_| anyhow::anyhow!("corpus lock poisoned"))?;
            guard.storage_dir().clone()
        };

        if let Some(dir) = dir
            && dir.exists()
        {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension() == Some("json".as_ref()) {
                    total_count += 1;

                    let Ok(json) = fs::read_to_string(&path) else {
                        parse_failed_count += 1;
                        continue;
                    };

                    let Ok(item) = serde_json::from_str::<CorpusItem>(&json) else {
                        parse_failed_count += 1;
                        continue;
                    };

                    let all_valid = item.calls.iter().all(|call| {
                        self.inner
                            .contract
                            .abi
                            .functions()
                            .any(|f| f.selector().as_slice() == call.selector)
                    });

                    if all_valid {
                        valid_count += 1;
                        let Ok(mut guard) = self.inner.corpus.write() else {
                            continue;
                        };
                        guard.pending.push(item);
                    } else {
                        invalid_call_count += 1;
                    }
                }
            }
        }

        Ok(CorpusStats {
            total_count,
            parse_failed_count,
            invalid_call_count,
            valid_count,
        })
    }

    /// Take a corpus item for execution.
    ///
    /// Returns a pending item if available. Otherwise picks a weighted
    /// random existing item, applies a random mutator, and returns the
    /// result. If the corpus is empty, a random sequence is generated.
    pub fn take(&self, rng: &mut fastrand::Rng, config: &Config) -> CorpusItem {
        let mut corpus = self.inner.corpus.write().unwrap();

        // 1. Pending replay
        if let Some(mut item) = corpus.pop_pending_item() {
            item.is_replay = true;
            return item;
        }

        // 2. Mutate an existing item
        let has_entries = !corpus.items.is_empty();
        if has_entries
            && rng.bool()
            && let Some((idx, mut item)) = corpus.random_item_for_mutation_with_index(rng)
        {
            if let Some(base) = corpus.items.get_mut(idx) {
                base.total_mutations += 1;
            }
            item.base_idx = Some(idx);
            let calls = &mut item.calls;
            drop(corpus);

            let m_idx = rng.usize(0..self.inner.mutators.len());
            let _ = self.inner.mutators[m_idx].mutate(rng, calls);
            return item;
        }
        drop(corpus);

        // 3. Generate a fresh random sequence
        CorpusItem::new(generate_random_sequence(&self.inner.selectors, rng, config))
    }

    /// Add an item to the corpus if it produces new coverage.
    ///
    /// Replay items that do not increase coverage are still promoted to
    /// the main corpus so they are available for future mutation.
    pub fn add(&self, item: CorpusItem, coverage: &LocalCoverage) -> bool {
        let Ok(mut guard) = self.inner.corpus.write() else {
            return false;
        };

        let added = guard.check_and_update_coverage(coverage, &item);
        if added {
            if let Some(idx) = item.base_idx
                && let Some(base) = guard.items.get_mut(idx)
            {
                base.new_finds_produced += 1;
            }
            return true;
        }

        if item.is_replay {
            let already_present = guard.items.iter().any(|i| i.calls == item.calls);
            if !already_present {
                guard.items.push(item);
                return true;
            }
        }

        false
    }

    /// Runtime statistics for the corpus.
    pub fn stats(&self) -> Stats {
        let Ok(guard) = self.inner.corpus.read() else {
            return Stats::default();
        };
        Stats {
            item_count: guard.items.len(),
            failure_count: guard.failures.len(),
            coverage_hits: guard.coverage().hit_count(),
        }
    }
}

fn generate_random_sequence(
    selectors: &[[u8; 4]],
    rng: &mut fastrand::Rng,
    config: &Config,
) -> Vec<Call> {
    let len = rng.usize(1..=config.sequence_length.max(1));
    let mut calls = Vec::with_capacity(len);
    for _ in 0..len {
        if selectors.is_empty() {
            break;
        }
        let sel_idx = rng.usize(0..selectors.len());
        let mut call = Call {
            selector: selectors[sel_idx],
            args: vec![0u8; 32 * 3],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        };
        if config.max_block_number_delay > 0 {
            call.block_number_delay = rng.u64(0..config.max_block_number_delay + 1);
        }
        if config.max_block_timestamp_delay > 0 {
            call.block_timestamp_delay = rng.u64(0..config.max_block_timestamp_delay + 1);
        }
        call.cap_delays();
        calls.push(call);
    }
    calls
}
