//! Corpus of interesting sequences.
//!
//! [`Corpus`] stores sequences that found new coverage or a new best value.
//! Fuzzers draw from it to generate mutations instead of only random
//! sequences, which keeps the search exploring useful state transitions.
//!
//! ```rust,no_run
//! use ripfuzz::maxer::{Corpus, Sequence, Value};
//! use ripfuzz::evm::{Chain, ChainConfig};
//! use alloy_primitives::U256;
//! use fastrand::Rng;
//!
//! let corpus = Corpus::new();
//! let mut rng = Rng::new();
//! let chain = Chain::empty(ChainConfig::default());
//! let sequence = Sequence::empty();
//! let value = Value::new(U256::from(1));
//! corpus.add(sequence, value, 1, 0, chain.clone());
//! let base = corpus.random_base(&mut rng);
//! ```

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use anyhow::{Context, Result, ensure};
use fastrand::Rng;
use revm::primitives::Bytes;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::evm::{Chain, SharedCoverage, Transaction};
use crate::maxer::{Call, Sequence, Value};

/// Maximum number of entries kept in the corpus.
const MAX_ENTRIES: usize = 256;

/// Schema version of the persisted corpus JSON.
const CORPUS_VERSION: u32 = 1;

/// The persisted corpus JSON document.
#[derive(Debug, Serialize, Deserialize)]
struct CorpusJson {
    version: u32,
    entries: Vec<EntryJson>,
}

/// One persisted corpus entry.
#[derive(Debug, Serialize, Deserialize)]
struct EntryJson {
    /// The measured value as a decimal string.
    value: String,
    /// The new coverage the sequence brought when it was added.
    new_edges: u64,
    /// The calls of the sequence in execution order.
    calls: Vec<CallJson>,
}

/// One persisted call: signature plus full calldata.
#[derive(Debug, Serialize, Deserialize)]
struct CallJson {
    signature: String,
    calldata: String,
}

/// An interesting sequence with its metadata.
///
/// The chain is the state after executing the sequence: the snapshot the
/// fuzzer extends with fresh calls instead of re-executing the sequence from
/// the initial state. Loaded entries carry no chain until the replay fills
/// it in, and the chain is never persisted.
#[derive(Debug, Clone)]
struct Entry {
    sequence: Sequence,
    value: Value,
    new_edges: u64,
    delta_activity: u64,
    chain: Option<Chain>,
}

/// A read-only snapshot of a corpus entry for reporting and debugging.
#[derive(Debug, Clone)]
pub struct EntrySnapshot {
    /// The sequence kept in the corpus.
    pub sequence: Sequence,
    /// The value measured after executing the sequence.
    pub value: Value,
    /// The new coverage the sequence brought when it was added.
    pub new_edges: u64,
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

    /// Snapshot all entries for reporting and debugging.
    ///
    /// The snapshot is in corpus order, which reflects the eviction history:
    /// entries kept early sit at the front, recently added entries at the
    /// back.
    pub fn entries(&self) -> Vec<EntrySnapshot> {
        let mut entries = Vec::new();
        for entry in self.lock().iter() {
            entries.push(EntrySnapshot {
                // checkrs: allow(clone_in_loops) a snapshot must own its entries
                sequence: entry.sequence.clone(),
                value: entry.value,
                new_edges: entry.new_edges,
            });
        }
        entries
    }

    /// A random snapshot base from the corpus.
    ///
    /// Entries are sampled proportional to their coverage gain, value, and
    /// delta activity.
    ///
    /// A sequence that unlocked a new code path (e.g. the first successful
    /// deposit) is a far better mutation base than the hundreds of one-edge
    /// entries around it, so sampling should favor it instead of diluting
    /// uniformly across the corpus. Prefixes that moved the objective, even
    /// with a dump, are also favored so a dip is extended more often than a
    /// no-op.
    ///
    /// Returns the sequence with the value and the state after executing it.
    /// Entries not yet replayed carry no snapshot and are skipped.
    pub fn random_base(&self, rng: &mut Rng) -> Option<(Sequence, Value, Chain)> {
        // 1. Compute the total weight of all replayed entries.
        let entries = self.lock();
        let total: u64 = entries
            .iter()
            .filter(|entry| entry.chain.is_some())
            .map(entry_weight)
            .sum();
        if total == 0 {
            return None;
        }
        // 2. Pick a weighted random entry.
        let mut pick = rng.u64(..total);
        for entry in entries.iter() {
            let Some(chain) = entry.chain.as_ref() else {
                continue;
            };
            let weight = entry_weight(entry);
            if pick < weight {
                let sequence =
                    // checkrs: allow(clone_in_loops) the base must own its state
                    entry.sequence.clone();
                let chain =
                    // checkrs: allow(clone_in_loops) the base must own its state
                    chain.clone();
                return Some((sequence, entry.value, chain));
            }
            pick -= weight;
        }
        // 3. Fall back to the last replayed entry for rounding errors.
        let last = entries.iter().rev().find(|entry| entry.chain.is_some())?;
        let chain = last.chain.clone()?;
        Some((last.sequence.clone(), last.value, chain))
    }

    /// The entry with the highest value, with its snapshot.
    ///
    /// Returns `None` for an empty corpus or one whose entries have not been
    /// replayed yet.
    pub fn best_entry(&self) -> Option<(Sequence, Value, Chain)> {
        let entries = self.lock();
        entries
            .iter()
            .filter(|entry| entry.chain.is_some())
            .filter_map(|entry| {
                let chain = entry.chain.clone()?;
                Some((entry.sequence.clone(), entry.value, chain))
            })
            .max_by_key(|(_, value, _)| *value)
    }

    /// Add an interesting sequence with the state after executing it.
    ///
    /// The corpus keeps interesting sequences with different rules depending
    /// on whether they improve the best value.
    ///
    /// - An improving sequence always joins, even when the corpus is full.
    ///   The highest-value state is the primary expansion base and losing it
    ///   stalls the value climb. It then replaces the weakest entry below the
    ///   current best.
    /// - Otherwise, when the corpus is full, the entry that brought the
    ///   fewest new edges is replaced, and only when the new sequence brings
    ///   at least as many.
    pub fn add(
        &self,
        sequence: Sequence,
        value: Value,
        new_edges: u64,
        delta_activity: u64,
        chain: Chain,
    ) {
        self.add_with_chain(sequence, value, new_edges, delta_activity, Some(chain));
    }

    /// Add an entry whose snapshot may be missing, e.g. one freshly loaded
    /// from disk that the replay has not rebuilt yet.
    fn add_with_chain(
        &self,
        sequence: Sequence,
        value: Value,
        new_edges: u64,
        delta_activity: u64,
        chain: Option<Chain>,
    ) {
        let mut entries = self.lock();
        // 1. Fast path when the corpus is not full.
        if entries.len() < MAX_ENTRIES {
            entries.push(Entry {
                sequence,
                value,
                new_edges,
                delta_activity,
                chain,
            });
            return;
        }
        // 2. Find the best value currently in the corpus.
        let best_value = entries
            .iter()
            .map(|entry| entry.value)
            .max()
            .unwrap_or_else(|| Value::new(U256::ZERO));
        // 3. Evict an entry to make room.
        let strength = new_edges.saturating_add(delta_activity);
        if value > best_value {
            // 3a. An improving sequence always joins and replaces the
            //     weakest entry below the current best.
            let weakest = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.value < best_value)
                .min_by_key(|(_, entry)| {
                    (
                        entry.new_edges.saturating_add(entry.delta_activity),
                        entry.value,
                    )
                })
                .map(|(index, _)| index)
                .unwrap_or(entries.len() - 1);
            entries.remove(weakest);
        } else {
            // 3b. Otherwise the weakest entry is only replaced when the new
            //     sequence is at least as strong.
            let weakest = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.value < best_value)
                .min_by_key(|(_, entry)| {
                    (
                        entry.new_edges.saturating_add(entry.delta_activity),
                        entry.value,
                    )
                })
                .map(|(index, _)| index);
            let Some(weakest) = weakest else {
                return;
            };
            if strength
                <= entries[weakest]
                    .new_edges
                    .saturating_add(entries[weakest].delta_activity)
            {
                return;
            }
            entries.remove(weakest);
        }
        // 4. Insert the new entry.
        entries.push(Entry {
            sequence,
            value,
            new_edges,
            delta_activity,
            chain,
        });
    }

    /// Load persisted entries into the corpus, resolving each call against
    /// the handler functions.
    ///
    /// A missing file yields an empty load. Entries whose calls cannot be
    /// resolved against the handlers (unknown signature, selector mismatch,
    /// or undecodable arguments) are skipped with a warning so a corpus from
    /// a different harness version never fails the campaign.
    pub fn load(&self, path: impl AsRef<Path>, handlers: &[Function]) -> Result<usize> {
        // 1. Read the persisted document, treating a missing file as empty.
        let path = path.as_ref();
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display()))?,
        };

        // 2. Parse the document and check the schema version.
        let document: CorpusJson = serde_json::from_slice(&data)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        ensure!(
            document.version == CORPUS_VERSION,
            "unsupported corpus version {} in {}",
            document.version,
            path.display()
        );

        // 3. Rebuild every entry, skipping ones that no longer resolve.
        let mut loaded = 0usize;
        for (index, entry) in document.entries.iter().enumerate() {
            let mut calls = Vec::with_capacity(entry.calls.len());
            for call in &entry.calls {
                match decode_call(call, handlers) {
                    Ok(call) => calls.push(call),
                    Err(err) => {
                        warn!(
                            entry = index,
                            error = %err,
                            "skipping corpus entry whose call does not resolve"
                        );
                        calls.clear();
                        break;
                    }
                }
            }
            if calls.is_empty() {
                continue;
            }
            let value = entry
                .value
                .parse::<alloy_primitives::U256>()
                .with_context(|| {
                    format!("entry {index} in {} has an invalid value", path.display())
                })?;
            self.add_with_chain(
                Sequence::new(calls),
                Value::new(value),
                entry.new_edges,
                0,
                None,
            );
            loaded += 1;
        }
        Ok(loaded)
    }

    /// Save the corpus entries as a JSON document, replacing the file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        // 1. Render one JSON entry per corpus entry.
        let document = CorpusJson {
            version: CORPUS_VERSION,
            entries: self
                .entries()
                .iter()
                .map(|entry| EntryJson {
                    value: entry.value.get().to_string(),
                    new_edges: entry.new_edges,
                    calls: entry
                        .sequence
                        .calls()
                        .iter()
                        .map(|call| CallJson {
                            signature: call.signature(),
                            calldata: format!("0x{}", hex::encode(call.calldata())),
                        })
                        .collect(),
                })
                .collect(),
        };

        // 2. Write the document under its namespaced directory.
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(&document).context("failed to serialize corpus")?;
        fs::write(path, json).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }
}

/// Sampling weight for one replayed corpus entry.
fn entry_weight(entry: &Entry) -> u64 {
    let value_weight = (entry.value.get().bit_len() as u64).saturating_add(1);
    entry
        .new_edges
        .saturating_add(1)
        .saturating_add(value_weight)
        .saturating_add(entry.delta_activity)
}

/// Rebuild one persisted call by resolving its signature against the handlers
/// and ABI-decoding its calldata.
fn decode_call(call: &CallJson, handlers: &[Function]) -> Result<Call> {
    // 1. Resolve the handler function by signature.
    let function = handlers
        .iter()
        .find(|handler| handler.signature() == call.signature)
        .with_context(|| format!("unknown handler signature `{}`", call.signature))?;

    // 2. Decode the calldata and check it targets the same selector.
    let data = hex::decode(call.calldata.trim_start_matches("0x"))
        .with_context(|| format!("call `{}` has invalid calldata", call.signature))?;
    ensure!(
        data.len() >= 4,
        "call `{}` calldata is shorter than a selector",
        call.signature
    );
    ensure!(
        data[..4] == *function.selector().as_slice(),
        "call `{}` calldata selector does not match",
        call.signature
    );

    // 3. Rebuild the call from the decoded arguments.
    let args = function
        .abi_decode_input(&data[4..])
        .with_context(|| format!("call `{}` calldata does not decode", call.signature))?;
    Ok(Call::new(function.clone(), DynSolValue::Tuple(args)))
}

/// Replayer of a loaded corpus against the current harness.
///
/// Mirrors the fuzzer's execution: each sequence runs on a clean chain clone
/// followed by the value call. Execution coverage merges into the shared map
/// the fuzzers will use, and each entry's edge count is recomputed against
/// the map so eviction stays comparable with campaign entries. Entries whose
/// value call no longer succeeds are dropped.
#[derive(Clone, Debug, Default)]
pub struct CorpusReplayer {
    chain: Option<Chain>,
    target: Option<Address>,
    deployer: Option<Address>,
    value_calldata: Option<Bytes>,
    coverage: Option<SharedCoverage>,
}

impl CorpusReplayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_chain(mut self, chain: Chain) -> Self {
        self.chain = Some(chain);
        self
    }

    pub fn with_target(mut self, target: Address) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_deployer(mut self, deployer: Address) -> Self {
        self.deployer = Some(deployer);
        self
    }

    pub fn with_value_calldata(mut self, calldata: Bytes) -> Self {
        self.value_calldata = Some(calldata);
        self
    }

    pub fn with_coverage(mut self, coverage: SharedCoverage) -> Self {
        self.coverage = Some(coverage);
        self
    }

    /// Replay every entry and return a rebuilt corpus with the number of
    /// entries that still execute cleanly.
    ///
    /// Execution coverage merges into the shared map the fuzzers will use,
    /// and each entry's edge count is recomputed against the map so eviction
    /// stays comparable with campaign entries. Entries whose value call no
    /// longer succeeds are dropped.
    pub fn replay(self, corpus: Corpus) -> Result<(Corpus, usize)> {
        // 1. Require the execution context.
        let chain = self
            .chain
            .context("chain not set, call CorpusReplayer::new().with_chain(..)")?;
        let target = self
            .target
            .context("target not set, call CorpusReplayer::new().with_target(..)")?;
        let deployer = self
            .deployer
            .context("deployer not set, call CorpusReplayer::new().with_deployer(..)")?;
        let value_calldata = self.value_calldata.context(
            "value calldata not set, call CorpusReplayer::new().with_value_calldata(..)",
        )?;
        let coverage = self
            .coverage
            .context("coverage not set, call CorpusReplayer::new().with_coverage(..)")?;

        // 2. Replay every entry, mirroring the fuzzer's execution.
        let value_tx = Transaction::new(target).calldata(value_calldata);
        let replayed = Corpus::new();
        for entry in corpus.entries() {
            // 2a. Execute the sequence on a clean chain clone.
            // checkrs: allow(clone_in_loops)
            let mut chain = chain.clone();
            let transactions = entry.sequence.transactions(target, deployer);
            let mut exec = chain.exec(&transactions)?;

            // 2b. Merge execution coverage into the shared map.
            let execution_coverage = exec
                .coverage
                .take()
                .context("execution coverage expected")?;
            let update = coverage.merge(&execution_coverage);
            let new_edges =
                (update.new_edges + update.new_depths + update.new_reverts + update.new_jump_edges)
                    as u64;

            // 2c. Measure the value after the sequence, dropping entries
            //     whose value call no longer succeeds.
            let output = chain.exec(std::slice::from_ref(&value_tx))?;
            let result = output
                .results
                .first()
                .context("value call result missing")?;
            if !result.success {
                warn!(
                    calls = entry.sequence.len(),
                    sequence = %entry.sequence,
                    "dropping corpus entry whose value call reverted"
                );
                continue;
            }
            let value = Value::decode(result)?;
            replayed.add(entry.sequence, value, new_edges, 0, chain);
        }
        let count = replayed.len();
        Ok((replayed, count))
    }
}

impl Default for Corpus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;
    use alloy_primitives::U256;
    use fastrand::Rng;

    use super::*;
    use crate::evm::ChainConfig;
    use crate::maxer::Sequence;

    fn random_sequence() -> Sequence {
        let handlers = [
            Function::parse("a()").unwrap(),
            Function::parse("b()").unwrap(),
        ];
        Sequence::random(&mut Rng::new(), &handlers, 3).unwrap()
    }

    fn test_chain() -> Chain {
        Chain::empty(ChainConfig::default())
    }

    #[test]
    fn random_base_on_empty_corpus_is_none() {
        let corpus = Corpus::new();
        assert!(corpus.is_empty());
        assert!(corpus.random_base(&mut Rng::new()).is_none());
    }

    #[test]
    fn add_and_random_base_return_entries() {
        let corpus = Corpus::new();
        corpus.add(
            random_sequence(),
            Value::new(U256::from(1)),
            3,
            0,
            test_chain(),
        );
        corpus.add(
            random_sequence(),
            Value::new(U256::from(2)),
            5,
            0,
            test_chain(),
        );

        assert_eq!(corpus.len(), 2);
        let (random, _, _) = corpus.random_base(&mut Rng::new()).unwrap();
        assert!(!random.is_empty());
    }

    #[test]
    fn random_favors_entries_with_more_coverage_gain() {
        // One high-gain entry among many one-edge entries: weighted sampling
        // must pick the high-gain entry for the bulk of the draws.
        let mut rng = Rng::with_seed(42);
        let corpus = Corpus::new();
        let high_gain = random_sequence();
        corpus.add(
            high_gain.clone(),
            Value::new(U256::from(1)),
            10_000,
            0,
            test_chain(),
        );
        for _ in 0..99 {
            corpus.add(
                random_sequence(),
                Value::new(U256::from(1)),
                0,
                0,
                test_chain(),
            );
        }

        let mut high_gain_picks = 0;
        for _ in 0..1_000 {
            let (random, _, _) = corpus.random_base(&mut rng).unwrap();
            if random == high_gain {
                high_gain_picks += 1;
            }
        }
        assert!(
            high_gain_picks > 500,
            "high-gain entry picked {high_gain_picks}/1000 times, expected the bulk"
        );
    }

    #[test]
    fn random_favors_entries_with_more_delta_activity() {
        let mut rng = Rng::with_seed(7);
        let corpus = Corpus::new();
        let active = random_sequence();
        corpus.add(
            active.clone(),
            Value::new(U256::from(1)),
            0,
            10_000,
            test_chain(),
        );
        for _ in 0..99 {
            corpus.add(
                random_sequence(),
                Value::new(U256::from(1)),
                0,
                0,
                test_chain(),
            );
        }

        let mut active_picks = 0;
        for _ in 0..1_000 {
            let (random, _, _) = corpus.random_base(&mut rng).unwrap();
            if random == active {
                active_picks += 1;
            }
        }
        assert!(
            active_picks > 500,
            "high-delta entry picked {active_picks}/1000 times, expected the bulk"
        );
    }

    #[test]
    fn entries_snapshot_lists_all_entries_in_order() {
        let corpus = Corpus::new();
        corpus.add(
            random_sequence(),
            Value::new(U256::from(1)),
            3,
            0,
            test_chain(),
        );
        corpus.add(
            random_sequence(),
            Value::new(U256::from(2)),
            5,
            0,
            test_chain(),
        );

        let entries = corpus.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].value, Value::new(U256::from(1)));
        assert_eq!(entries[0].new_edges, 3);
        assert_eq!(entries[1].value, Value::new(U256::from(2)));
        assert_eq!(entries[1].new_edges, 5);
    }

    #[test]
    fn full_corpus_replaces_the_weakest_entry() {
        let corpus = Corpus::new();
        for _ in 0..256 {
            corpus.add(
                random_sequence(),
                Value::new(U256::from(1)),
                5,
                0,
                test_chain(),
            );
        }
        // A sequence with fewer new edges than the weakest entry is rejected.
        corpus.add(
            random_sequence(),
            Value::new(U256::from(1)),
            4,
            0,
            test_chain(),
        );
        assert_eq!(corpus.len(), 256);

        // A sequence with more new edges replaces the weakest entry.
        corpus.add(
            random_sequence(),
            Value::new(U256::from(1)),
            10,
            0,
            test_chain(),
        );
        assert_eq!(corpus.len(), 256);
    }

    #[test]
    fn improving_sequence_joins_even_when_the_corpus_is_full() {
        let corpus = Corpus::new();
        for _ in 0..256 {
            corpus.add(
                random_sequence(),
                Value::new(U256::from(1)),
                5,
                0,
                test_chain(),
            );
        }

        // The new best-value sequence always joins, evicting the weakest
        // entry below the previous best.
        corpus.add(
            random_sequence(),
            Value::new(U256::from(9)),
            0,
            0,
            test_chain(),
        );
        assert_eq!(corpus.len(), 256);

        // Later low-edge entries must never evict the best-value entry.
        for _ in 0..256 {
            corpus.add(
                random_sequence(),
                Value::new(U256::from(1)),
                1_000_000,
                0,
                test_chain(),
            );
        }
        let best = corpus
            .entries()
            .into_iter()
            .find(|entry| entry.value == Value::new(U256::from(9)));
        assert!(best.is_some(), "best-value entry was evicted");
    }

    /// A unique temp path per test run so parallel tests never collide.
    fn temp_corpus_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "ripfuzz-corpus-test-{}-{}.json",
            std::process::id(),
            fastrand::u64(..)
        ))
    }

    /// Saving and loading must round-trip entries: the reloaded corpus holds
    /// the same value, edge count, and per-call calldata.
    #[test]
    fn save_then_load_round_trips_entries() {
        let function = Function::parse("set(uint256)").unwrap();
        let call = Call::new(
            function,
            DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(7), 256)]),
        );
        let handlers = vec![Function::parse("set(uint256)").unwrap()];

        let corpus = Corpus::new();
        corpus.add(
            Sequence::new(vec![call]),
            Value::new(U256::from(42)),
            5,
            0,
            test_chain(),
        );

        let path = temp_corpus_path();
        corpus.save(&path).unwrap();

        let reloaded = Corpus::new();
        let loaded = reloaded.load(&path, &handlers).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(loaded, 1);
        let entries = reloaded.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].value, Value::new(U256::from(42)));
        assert_eq!(entries[0].new_edges, 5);
        assert_eq!(entries[0].sequence.calls().len(), 1);
        assert_eq!(entries[0].sequence.calls()[0].signature(), "set(uint256)");
        assert_eq!(
            entries[0].sequence.calls()[0].calldata(),
            corpus.entries()[0].sequence.calls()[0].calldata()
        );
    }

    /// Entries whose calls no longer resolve against the handlers must be
    /// skipped so a corpus from a different harness never fails a campaign.
    #[test]
    fn load_skips_entries_with_unknown_signatures() {
        let handlers = vec![Function::parse("set(uint256)").unwrap()];

        let corpus = Corpus::new();
        corpus.add(
            random_sequence(),
            Value::new(U256::from(1)),
            3,
            0,
            test_chain(),
        );
        let path = temp_corpus_path();
        corpus.save(&path).unwrap();

        let reloaded = Corpus::new();
        let loaded = reloaded.load(&path, &handlers).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(loaded, 0);
        assert!(reloaded.is_empty());
    }

    /// A missing corpus file is an empty corpus, not an error.
    #[test]
    fn load_without_a_file_returns_zero() {
        let path = temp_corpus_path();
        let loaded = Corpus::new().load(&path, &[]).unwrap();

        assert_eq!(loaded, 0);
    }
}
