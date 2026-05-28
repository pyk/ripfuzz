//! Thread-safe shared corpus.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use rayon::prelude::*;

use alloy_dyn_abi::{DynSolValue, Specifier};
use alloy_json_abi::StateMutability;
use anyhow::{Result, ensure};
use tracing::debug;

use crate::fuzzer::corpus::random::RandomDynSolValue;
use crate::fuzzer::corpus::{Call, Config, ExtractedLiterals, Item};

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
}

/// Combined in-memory store for corpus items.
///
/// The `map` provides O(1) deduplication by [`Item::id`]; the `vec`
/// provides O(1) random sampling. Both are updated together atomically
/// under a single [`parking_lot::RwLock`].
pub struct SharedCorpusItems {
    pub ids: HashSet<String>,
    pub vec: Vec<Item>,
}

impl SharedCorpusItems {
    /// Add an item to the random-access vector if its id is not already
    /// present in the deduplication set.
    ///
    /// Returns `true` when the item was newly inserted, `false` if it
    /// already existed.
    pub fn try_add(&mut self, item: Item) -> bool {
        let id = item.id();
        if self.ids.insert(id) {
            self.vec.push(item);
            true
        } else {
            false
        }
    }
}

pub struct SharedCorpusInner {
    pub corpus_dir: PathBuf,
    pub items: RwLock<SharedCorpusItems>,
    pub target_functions: Vec<alloy_json_abi::Function>,
    pub max_calls_length: usize,
    pub literals: ExtractedLiterals,
}

impl std::fmt::Debug for SharedCorpusInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.items.read().vec.len();
        f.debug_struct("SharedCorpusInner")
            .field("items", &format_args!("[{len} items]"))
            .field(
                "target_functions",
                &format_args!("[{} functions]", self.target_functions.len()),
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

/// Compute the namespaced corpus directory for a given artifact.
///
/// Only the file name component of `artifact_id.path` is used, so
/// absolute source paths do not pollute the on-disk layout.
pub fn get_dir(base: impl AsRef<Path>, artifact_id: &crate::foundry::ArtifactId) -> PathBuf {
    let file_name = artifact_id
        .path
        .file_name()
        .map(|f| f.to_os_string())
        .unwrap_or_else(|| artifact_id.path.as_os_str().to_os_string());
    base.as_ref().join(file_name).join(&artifact_id.name)
}

impl SharedCorpus {
    /// Create an empty corpus from a [`Config`].
    ///
    /// `config.corpus_dir` should already be namespaced by artifact (use
    /// [`get_dir`] to compute it). No disk I/O is performed until
    /// [`Self::load_items`] is called.
    pub fn new(config: Config) -> Self {
        let inner = Arc::new(SharedCorpusInner {
            corpus_dir: config.corpus_dir,
            items: RwLock::new(SharedCorpusItems {
                ids: HashSet::new(),
                vec: Vec::new(),
            }),
            target_functions: config.target_functions,
            max_calls_length: config.max_calls_length,
            literals: config.literals,
        });

        Self { inner }
    }

    /// Load corpus items from the storage directory and validate them
    /// against the target functions.
    ///
    /// Valid items are added to the pending queue. Invalid or unparsable
    /// items are counted in the returned [`CorpusStats`] but are not stored.
    pub fn load_items(&self) -> Result<CorpusStats> {
        let dir = &self.inner.corpus_dir;
        if !dir.exists() {
            return Ok(CorpusStats::default());
        }

        // Phase 1: collect all JSON file paths in the namespaced dir.
        let paths: Vec<PathBuf> = walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension() == Some("json".as_ref()))
            .map(|e| e.path().to_path_buf())
            .collect();

        let total_count = paths.len();

        // Phase 2: read + parse + validate + insert in parallel.
        // Each thread inserts valid items directly under the write lock.
        let (parse_failed_count, invalid_call_count) = paths
            .into_par_iter()
            .map(|path| {
                let json = match fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(_) => return (1, 0),
                };
                let item = match serde_json::from_str::<Item>(&json) {
                    Ok(i) => i,
                    Err(_) => return (1, 0),
                };
                let all_valid = item.calls.iter().all(|call| {
                    self.inner
                        .target_functions
                        .iter()
                        .any(|f| f.signature() == call.function.signature())
                });
                if all_valid {
                    let mut items = self.inner.items.write();
                    items.try_add(item);
                    (0, 0)
                } else {
                    (0, 1)
                }
            })
            .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));

        let valid_count = total_count - parse_failed_count - invalid_call_count;

        Ok(CorpusStats {
            total_count,
            parse_failed_count,
            invalid_call_count,
            valid_count,
        })
    }

    /// Return a cloned snapshot of all items currently in the corpus.
    pub fn items(&self) -> Vec<Item> {
        self.inner.items.read().vec.clone()
    }

    /// Select a random existing item from the corpus.
    ///
    /// Used when `is_fresh_item` returns `false`.
    pub fn pick_item(&self, rng: &mut fastrand::Rng) -> Result<Item> {
        let items = self.inner.items.read();
        let count = items.vec.len();
        ensure!(
            count > 0,
            "pick_item called on empty corpus - check is_fresh_item first"
        );
        let idx = rng.usize(0..count);
        Ok(items.vec[idx].clone())
    }

    /// Add a corpus item to the collection.
    ///
    /// Deduplicates by [`Item::id`]. Returns `Ok(())` whether the item
    /// already exists or was newly inserted. Only the thread that wins the
    /// atomic insert performs disk I/O, so the same item is never written
    /// twice even under concurrent calls.
    pub fn add_item(&self, item: Item) -> Result<()> {
        // Only the first thread to successfully insert reaches the
        // disk-write code below. All other racing threads see the
        // existing key and return early.
        let newly_inserted = {
            let mut items = self.inner.items.write();
            items.try_add(item.clone())
        };
        if !newly_inserted {
            return Ok(());
        }

        debug!(item_id = %item.id(), "corpus item added");

        // Write to disk
        let path = self.inner.corpus_dir.join(format!("{}.json", item.id()));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(&item)?;
        fs::write(&path, json)?;

        Ok(())
    }

    /// Runtime statistics for the corpus.
    pub fn stats(&self) -> Stats {
        let item_count = self.inner.items.read().vec.len();
        Stats {
            item_count,
            failure_count: 0,
        }
    }

    /// Decide whether the next input should be a freshly generated sequence.
    ///
    /// Returns `true` when the corpus is empty, or with 30% probability
    /// otherwise. Returns `false` the remaining 70% of the time, signalling
    /// that the caller should reuse an existing corpus item.
    pub fn is_fresh_item(&self, rng: &mut fastrand::Rng) -> bool {
        if self.inner.items.read().vec.is_empty() {
            return true;
        }
        rng.f32() < 0.30
    }

    /// Generate a single random call for the target contract.
    fn generate_call(&self, rng: &mut fastrand::Rng) -> Call {
        let functions = &self.inner.target_functions;
        if functions.is_empty() {
            return Call::default();
        }
        let idx = rng.usize(0..functions.len());
        let func = &functions[idx];
        let values: Vec<DynSolValue> = func
            .inputs
            .iter()
            .filter_map(|p| p.resolve().ok())
            .map(|ty| ty.random(rng, &self.inner.literals))
            .collect();
        let value = if func.state_mutability == StateMutability::Payable {
            Some(crate::fuzzer::corpus::random::random_uint(
                rng,
                256,
                &self.inner.literals,
            ))
        } else {
            None
        };

        Call {
            // checkrs: allow(clone_in_loops)
            function: func.clone(),
            args: DynSolValue::Tuple(values),
            value,
            ..Default::default()
        }
    }

    /// Generate a fresh corpus item from scratch.
    ///
    /// Uses the supplied RNG to decide sequence length, function selection,
    /// and argument values. Each thread with a distinct seeded RNG will
    /// produce a different item.
    pub fn generate_item(&self, rng: &mut fastrand::Rng) -> Item {
        let max_len = self.inner.max_calls_length.max(1);
        let len = rng.usize(1..=max_len);
        let mut calls = Vec::with_capacity(len);

        for _ in 0..len {
            calls.push(self.generate_call(rng));
        }

        Item::from(calls)
    }

    /// Insert a randomly generated contract call at a random position in the
    /// sequence.
    ///
    /// Returns an error if the item already contains the maximum allowed
    /// number of calls.
    pub fn add_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            item.calls.len() < self.inner.max_calls_length,
            "item already contains max_calls_length calls"
        );
        let pos = rng.usize(0..=item.calls.len());
        item.calls.insert(pos, self.generate_call(rng));
        Ok(())
    }

    /// Remove a contract call at a random position in the sequence.
    ///
    /// Returns an error if the item contains only a single call.
    pub fn remove_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            item.calls.len() > 1,
            "item must contain at least one call to remove"
        );
        let pos = rng.usize(0..item.calls.len());
        item.calls.remove(pos);
        Ok(())
    }

    /// Swap two contract calls at distinct random positions in the
    /// sequence.
    ///
    /// The sequence length is preserved; only the order of existing calls
    /// changes. The two chosen positions are always different, so the
    /// mutation is never a no-op.
    pub fn swap_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        let len = item.calls.len();
        ensure!(len > 1, "item must contain at least two calls to swap");
        let a = rng.usize(0..len);
        let b = (a + rng.usize(1..len)) % len;
        item.calls.swap(a, b);
        Ok(())
    }

    /// Replace a contract call at a random position with a freshly
    /// generated random call.
    ///
    /// The sequence length is preserved.
    pub fn replace_call(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            !item.calls.is_empty(),
            "item must contain at least one call"
        );
        let pos = rng.usize(0..item.calls.len());
        item.calls[pos] = self.generate_call(rng);
        Ok(())
    }

    /// Regenerate the argument values for a contract call at a random
    /// position.
    ///
    /// The function signature, caller, and value are preserved; only the
    /// `args` field is replaced with fresh random values.
    pub fn update_args(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        ensure!(
            !item.calls.is_empty(),
            "item must contain at least one call"
        );
        let pos = rng.usize(0..item.calls.len());
        let call = &mut item.calls[pos];
        let values: Vec<DynSolValue> = call
            .function
            .inputs
            .iter()
            .filter_map(|p| p.resolve().ok())
            .map(|ty| ty.random(rng, &self.inner.literals))
            .collect();
        call.args = DynSolValue::Tuple(values);
        Ok(())
    }

    /// Regenerate the `value` field for a payable contract call at a
    /// random position.
    ///
    /// Only calls whose function has `StateMutability::Payable` are
    /// eligible. The sequence length is preserved.
    pub fn update_value(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        let payable: Vec<usize> = item
            .calls
            .iter()
            .enumerate()
            .filter(|(_, c)| c.function.state_mutability == StateMutability::Payable)
            .map(|(i, _)| i)
            .collect();
        ensure!(!payable.is_empty(), "item contains no payable calls");
        let pos = payable[rng.usize(0..payable.len())];
        item.calls[pos].value = Some(crate::fuzzer::corpus::random::random_uint(
            rng,
            256,
            &self.inner.literals,
        ));
        Ok(())
    }

    /// Apply a randomly selected applicable mutation to the item.
    ///
    /// Builds a stack-only list of mutations that are legal for the
    /// current item state, picks one uniformly, and executes it. If no
    /// mutation is applicable the item is left unchanged.
    pub fn mutate_item(&self, rng: &mut fastrand::Rng, item: &mut Item) -> Result<()> {
        let mut ops = [0u8; 6];
        let mut count = 0usize;

        if item.calls.len() < self.inner.max_calls_length {
            ops[count] = 0;
            count += 1;
        }
        if item.calls.len() > 1 {
            ops[count] = 1;
            count += 1;
            ops[count] = 2;
            count += 1;
        }
        if !item.calls.is_empty() {
            ops[count] = 3;
            count += 1;
            ops[count] = 4;
            count += 1;
        }
        if item
            .calls
            .iter()
            .any(|c| c.function.state_mutability == StateMutability::Payable)
        {
            ops[count] = 5;
            count += 1;
        }

        ensure!(count > 0, "no applicable mutations for this item");

        match ops[rng.usize(0..count)] {
            0 => self.add_call(rng, item),
            1 => self.remove_call(rng, item),
            2 => self.swap_call(rng, item),
            3 => self.replace_call(rng, item),
            4 => self.update_args(rng, item),
            5 => self.update_value(rng, item),
            _ => unreachable!(),
        }
    }

    /// Get the next corpus item for execution.
    ///
    /// Returns a freshly generated item 30% of the time (or when the corpus
    /// is empty). The remaining 70% of the time picks a random existing
    /// item and mutates it before returning.
    pub fn next_item(&self, rng: &mut fastrand::Rng) -> Item {
        if self.is_fresh_item(rng) {
            return self.generate_item(rng);
        }
        let mut item = match self.pick_item(rng) {
            Ok(item) => item,
            Err(_) => unreachable!("is_fresh_item guarantees non-empty corpus"),
        };
        match self.mutate_item(rng, &mut item) {
            Ok(()) => item,
            Err(_) => unreachable!("applicability filter guarantees success"),
        }
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Barrier;

    use crate::evm::Contract;

    use super::*;

    #[test]
    fn parallel_add_writes_once() {
        let tmp = tempfile::tempdir().unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(4)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        let item = Item::from(vec![Call {
            function: alloy_json_abi::Function::parse("foo(uint256)").unwrap(),
            args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                alloy_primitives::U256::ZERO,
                256,
            )]),
            ..Default::default()
        }]);

        let threads = 16;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);

        for _ in 0..threads {
            let corpus = corpus.clone();
            let item = item.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                corpus.add_item(item)
            }));
        }

        for handle in handles {
            handle.join().unwrap().expect("add should succeed");
        }

        assert_eq!(
            corpus.inner.items.read().ids.len(),
            1,
            "exactly one item should be in the set"
        );
        assert_eq!(
            corpus.inner.items.read().vec.len(),
            1,
            "exactly one item should be in the vec"
        );

        let expected_path = corpus.inner.corpus_dir.join(format!("{}.json", item.id()));
        assert!(
            expected_path.exists(),
            "corpus item should be written to disk at {:?}",
            expected_path
        );

        // Ensure there is exactly one file in the namespaced tree.
        let file_count: usize = walkdir::WalkDir::new(&corpus.inner.corpus_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .count();
        assert_eq!(file_count, 1, "only one file should exist on disk");
    }

    #[test]
    fn parallel_generate_item_produces_unique_items() {
        let tmp = tempfile::tempdir().unwrap();
        let func_a = alloy_json_abi::Function::parse("foo(uint256)").unwrap();
        let func_b = alloy_json_abi::Function::parse("bar(address,bool)").unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![func_a, func_b],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(8)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        let threads = 16;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);

        for i in 0..threads {
            let corpus = corpus.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let mut rng = fastrand::Rng::with_seed(i as u64);
                barrier.wait();
                let item = corpus.generate_item(&mut rng);
                item.id()
            }));
        }

        let mut ids = Vec::with_capacity(threads);
        for handle in handles {
            ids.push(handle.join().unwrap());
        }

        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(
            unique.len(),
            threads,
            "expected all {threads} generated items to be unique"
        );
    }

    #[test]
    fn parallel_pick_item_selects_diverse_items() {
        let tmp = tempfile::tempdir().unwrap();
        let func = alloy_json_abi::Function::parse("foo(uint256)").unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![func],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(4)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        // Seed the corpus with 20 unique items.
        for i in 0..20 {
            let item = Item::from(vec![Call {
                function: alloy_json_abi::Function::parse("foo(uint256)").unwrap(),
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                    alloy_primitives::U256::from(i),
                    256,
                )]),
                ..Default::default()
            }]);
            corpus.add_item(item).unwrap();
        }

        let threads = 16;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);

        for t in 0..threads {
            let corpus = corpus.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let mut rng = fastrand::Rng::with_seed(t as u64);
                barrier.wait();
                let item = corpus.pick_item(&mut rng).unwrap();
                item.id()
            }));
        }

        let mut ids = Vec::with_capacity(threads);
        for handle in handles {
            ids.push(handle.join().unwrap());
        }

        let unique: HashSet<&String> = ids.iter().collect();
        assert!(
            unique.len() >= 10,
            "expected at least 10 unique items picked, got {} / {threads}",
            unique.len()
        );
    }

    #[test]
    fn is_fresh_item_returns_true_when_corpus_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(4)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);
        let mut rng = fastrand::Rng::with_seed(42);

        assert!(corpus.is_fresh_item(&mut rng));
    }

    #[test]
    fn is_fresh_item_distribution_around_thirty_percent() {
        let tmp = tempfile::tempdir().unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(4)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);
        let item = Item::from(vec![Call {
            function: alloy_json_abi::Function::parse("foo(uint256)").unwrap(),
            args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                alloy_primitives::U256::ZERO,
                256,
            )]),
            ..Default::default()
        }]);
        corpus.add_item(item).unwrap();

        let mut rng = fastrand::Rng::with_seed(123);
        let mut true_count = 0usize;
        let total = 1_000usize;

        for _ in 0..total {
            if corpus.is_fresh_item(&mut rng) {
                true_count += 1;
            }
        }

        assert!(
            true_count > 200 && true_count < 400,
            "expected ~30% true, got {true_count} / {total}"
        );
    }

    #[test]
    fn parallel_add_call_produces_unique_items() {
        let tmp = tempfile::tempdir().unwrap();
        let func = alloy_json_abi::Function::parse("foo(uint256)").unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![func],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let max_calls = 64;
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(max_calls)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        // Seed the corpus with one item containing 32 identical calls.
        let base_call = Call {
            function: alloy_json_abi::Function::parse("foo(uint256)").unwrap(),
            args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                alloy_primitives::U256::ZERO,
                256,
            )]),
            ..Default::default()
        };
        let base_item = Item::from(vec![base_call; 32]);
        let original_id = base_item.id();
        corpus.add_item(base_item.clone()).unwrap();

        let threads = 16;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);

        for t in 0..threads {
            let corpus = corpus.clone();
            let item = base_item.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let mut rng = fastrand::Rng::with_seed(t as u64);
                let mut item = item;
                barrier.wait();
                corpus
                    .add_call(&mut rng, &mut item)
                    .expect("add_call should succeed");
                assert_eq!(
                    item.calls.len(),
                    33,
                    "mutated item must have exactly 33 calls (32 + 1)"
                );
                corpus.add_item(item.clone()).expect("add should succeed");
                item
            }));
        }

        let mut items = Vec::with_capacity(threads);
        for handle in handles {
            items.push(handle.join().unwrap());
        }

        // All mutated items must differ from the original.
        for item in &items {
            assert_ne!(
                item.id(),
                original_id,
                "mutated item must differ from original ({original_id})"
            );
        }

        // All mutated items must be unique among themselves.
        let unique: HashSet<String> = items.iter().map(|i| i.id()).collect();
        assert!(
            unique.len() >= threads - 1,
            "expected at least {} unique mutated items, got {}",
            threads - 1,
            unique.len()
        );
    }

    #[test]
    fn parallel_remove_call_produces_unique_items() {
        let tmp = tempfile::tempdir().unwrap();
        let func = alloy_json_abi::Function::parse("foo(uint256)").unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![func],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(64)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        // Seed the corpus with one item containing 32 distinct calls.
        let mut calls = Vec::with_capacity(32);
        for i in 0..32 {
            calls.push(Call {
                function: alloy_json_abi::Function::parse("foo(uint256)").unwrap(),
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                    alloy_primitives::U256::from(i),
                    256,
                )]),
                ..Default::default()
            });
        }
        let base_item = Item::from(calls);
        let original_id = base_item.id();
        corpus.add_item(base_item.clone()).unwrap();

        let threads = 16;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);

        for t in 0..threads {
            let corpus = corpus.clone();
            let item = base_item.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let mut rng = fastrand::Rng::with_seed(t as u64);
                let mut item = item;
                barrier.wait();
                corpus
                    .remove_call(&mut rng, &mut item)
                    .expect("remove_call should succeed");
                assert_eq!(
                    item.calls.len(),
                    31,
                    "mutated item must have exactly 31 calls (32 - 1)"
                );
                corpus.add_item(item.clone()).expect("add should succeed");
                item
            }));
        }

        let mut items = Vec::with_capacity(threads);
        for handle in handles {
            items.push(handle.join().unwrap());
        }

        // All mutated items must differ from the original.
        for item in &items {
            assert_ne!(
                item.id(),
                original_id,
                "mutated item must differ from original ({original_id})"
            );
        }
    }

    #[test]
    fn parallel_swap_call_produces_unique_items() {
        let tmp = tempfile::tempdir().unwrap();
        let func = alloy_json_abi::Function::parse("foo(uint256)").unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![func],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(64)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        // Seed the corpus with one item containing 32 distinct calls.
        let mut calls = Vec::with_capacity(32);
        for i in 0..32 {
            calls.push(Call {
                function: alloy_json_abi::Function::parse("foo(uint256)").unwrap(),
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                    alloy_primitives::U256::from(i),
                    256,
                )]),
                ..Default::default()
            });
        }
        let base_item = Item::from(calls);
        let original_id = base_item.id();
        corpus.add_item(base_item.clone()).unwrap();

        let threads = 16;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);

        for t in 0..threads {
            let corpus = corpus.clone();
            let item = base_item.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let mut rng = fastrand::Rng::with_seed(t as u64);
                let mut item = item;
                barrier.wait();
                corpus
                    .swap_call(&mut rng, &mut item)
                    .expect("swap_call should succeed");
                assert_eq!(
                    item.calls.len(),
                    32,
                    "mutated item must still have exactly 32 calls"
                );
                corpus.add_item(item.clone()).expect("add should succeed");
                item
            }));
        }

        let mut items = Vec::with_capacity(threads);
        for handle in handles {
            items.push(handle.join().unwrap());
        }

        // All mutated items must differ from the original.
        for item in &items {
            assert_ne!(
                item.id(),
                original_id,
                "mutated item must differ from original ({original_id})"
            );
        }
    }

    #[test]
    fn parallel_replace_call_produces_unique_items() {
        let tmp = tempfile::tempdir().unwrap();
        let func_a = alloy_json_abi::Function::parse("foo(uint256)").unwrap();
        let func_b = alloy_json_abi::Function::parse("bar(address,bool)").unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![func_a, func_b],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(64)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        // Seed the corpus with one item containing 32 distinct calls.
        let mut calls = Vec::with_capacity(32);
        for i in 0..32 {
            calls.push(Call {
                function: alloy_json_abi::Function::parse("foo(uint256)").unwrap(),
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                    alloy_primitives::U256::from(i),
                    256,
                )]),
                ..Default::default()
            });
        }
        let base_item = Item::from(calls);
        let original_id = base_item.id();
        corpus.add_item(base_item.clone()).unwrap();

        let threads = 16;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);

        for t in 0..threads {
            let corpus = corpus.clone();
            let item = base_item.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let mut rng = fastrand::Rng::with_seed(t as u64);
                let mut item = item;
                barrier.wait();
                corpus
                    .replace_call(&mut rng, &mut item)
                    .expect("replace_call should succeed");
                assert_eq!(
                    item.calls.len(),
                    32,
                    "mutated item must still have exactly 32 calls"
                );
                corpus.add_item(item.clone()).expect("add should succeed");
                item
            }));
        }

        let mut items = Vec::with_capacity(threads);
        for handle in handles {
            items.push(handle.join().unwrap());
        }

        // All mutated items must differ from the original.
        for item in &items {
            assert_ne!(
                item.id(),
                original_id,
                "mutated item must differ from original ({original_id})"
            );
        }
    }

    #[test]
    fn parallel_update_args_produces_unique_items() {
        let tmp = tempfile::tempdir().unwrap();
        let func = alloy_json_abi::Function::parse("foo(uint256)").unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![func],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(64)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        // Seed the corpus with one item containing 32 distinct calls.
        let mut calls = Vec::with_capacity(32);
        for i in 0..32 {
            calls.push(Call {
                function: alloy_json_abi::Function::parse("foo(uint256)").unwrap(),
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                    alloy_primitives::U256::from(i),
                    256,
                )]),
                ..Default::default()
            });
        }
        let base_item = Item::from(calls);
        let original_id = base_item.id();
        corpus.add_item(base_item.clone()).unwrap();

        let threads = 16;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);

        for t in 0..threads {
            let corpus = corpus.clone();
            let item = base_item.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let mut rng = fastrand::Rng::with_seed(t as u64);
                let mut item = item;
                barrier.wait();
                corpus
                    .update_args(&mut rng, &mut item)
                    .expect("update_args should succeed");
                assert_eq!(
                    item.calls.len(),
                    32,
                    "mutated item must still have exactly 32 calls"
                );
                corpus.add_item(item.clone()).expect("add should succeed");
                item
            }));
        }

        let mut items = Vec::with_capacity(threads);
        for handle in handles {
            items.push(handle.join().unwrap());
        }

        // All mutated items must differ from the original.
        for item in &items {
            assert_ne!(
                item.id(),
                original_id,
                "mutated item must differ from original ({original_id})"
            );
        }
    }

    #[test]
    fn parallel_update_value_produces_unique_items() {
        let tmp = tempfile::tempdir().unwrap();
        let mut func_pay = alloy_json_abi::Function::parse("pay()").unwrap();
        func_pay.state_mutability = alloy_json_abi::StateMutability::Payable;
        let func_nonpay = alloy_json_abi::Function::parse("foo(uint256)").unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![func_pay.clone(), func_nonpay.clone()],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(64)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        // Seed the corpus with one item containing 32 calls,
        // every even index is payable.
        let mut calls = Vec::with_capacity(32);
        for i in 0..32 {
            let (func, value) = if i % 2 == 0 {
                (func_pay.clone(), Some(alloy_primitives::U256::from(i)))
            } else {
                (func_nonpay.clone(), None)
            };
            calls.push(Call {
                function: func,
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                    alloy_primitives::U256::from(i),
                    256,
                )]),
                value,
                ..Default::default()
            });
        }
        let base_item = Item::from(calls);
        let original_id = base_item.id();
        corpus.add_item(base_item.clone()).unwrap();

        let threads = 16;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);

        for t in 0..threads {
            let corpus = corpus.clone();
            let item = base_item.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let mut rng = fastrand::Rng::with_seed(t as u64);
                let mut item = item;
                barrier.wait();
                corpus
                    .update_value(&mut rng, &mut item)
                    .expect("update_value should succeed");
                assert_eq!(
                    item.calls.len(),
                    32,
                    "mutated item must still have exactly 32 calls"
                );
                corpus.add_item(item.clone()).expect("add should succeed");
                item
            }));
        }

        let mut items = Vec::with_capacity(threads);
        for handle in handles {
            items.push(handle.join().unwrap());
        }

        // All mutated items must differ from the original.
        for item in &items {
            assert_ne!(
                item.id(),
                original_id,
                "mutated item must differ from original ({original_id})"
            );
        }
    }

    #[test]
    fn mutate_item_produces_diverse_results() {
        let tmp = tempfile::tempdir().unwrap();
        let mut func_pay = alloy_json_abi::Function::parse("pay()").unwrap();
        func_pay.state_mutability = alloy_json_abi::StateMutability::Payable;
        let func_a = alloy_json_abi::Function::parse("foo(uint256)").unwrap();
        let func_b = alloy_json_abi::Function::parse("bar(address,bool)").unwrap();
        let contract = Contract {
            artifact_id: crate::foundry::ArtifactId {
                path: PathBuf::from("src/Test.sol"),
                name: "Test".into(),
            },
            abi: alloy_json_abi::JsonAbi::default(),
            target_functions: vec![func_pay.clone(), func_a, func_b],
            invariant_functions: vec![],
            setup_function: None,
            libraries: Vec::new(),
            initcode: "0x".into(),
        };

        let corpus_dir = get_dir(tmp.path(), &contract.artifact_id);
        let corpus_config = Config::new(corpus_dir)
            .target_functions(contract.target_functions.clone())
            .max_calls(64)
            .literals(ExtractedLiterals::default());
        let corpus = SharedCorpus::new(corpus_config);

        // Build a base item with 8 calls (mixed payable and non-payable).
        let mut calls = Vec::with_capacity(8);
        for i in 0..8 {
            let (func, value) = if i % 2 == 0 {
                (func_pay.clone(), Some(alloy_primitives::U256::from(i)))
            } else {
                (
                    alloy_json_abi::Function::parse("foo(uint256)").unwrap(),
                    None,
                )
            };
            calls.push(Call {
                function: func,
                args: alloy_dyn_abi::DynSolValue::Tuple(vec![alloy_dyn_abi::DynSolValue::Uint(
                    alloy_primitives::U256::from(i),
                    256,
                )]),
                value,
                ..Default::default()
            });
        }
        let base_item = Item::from(calls);
        let original_id = base_item.id();

        // Run mutate_item many times with different seeds.
        let mut ids = HashSet::new();
        for seed in 0..200usize {
            let mut rng = fastrand::Rng::with_seed(seed as u64);
            let mut item = base_item.clone();
            corpus
                .mutate_item(&mut rng, &mut item)
                .expect("mutate_item should succeed");
            ids.insert(item.id());
        }

        assert!(
            ids.len() > 10,
            "expected diverse mutations, got {} unique ids out of 200",
            ids.len()
        );
        assert!(
            !ids.contains(&original_id),
            "mutate_item should never return the unmodified original item"
        );
    }
}
