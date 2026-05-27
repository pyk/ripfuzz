//! Thread-safe corpus shared across parallel fuzzer threads.
//!
//! ## Separation of concerns
//!
//! `SharedCorpus` is responsible for:
//! - Loading and validating corpus from disk.
//! - Defining [`Item`] which is convertible to/from
//!   [`evm::chain::ExecInput`](crate::evm::chain::ExecInput).
//! - Serializing corpus items as compact JSON.
//! - Providing [`next`](SharedCorpus::next) to return a randomly selected
//!   corpus item (mutated when sourced from the existing pool) for a fuzzer
//!   thread.
//! - Providing [`add`](SharedCorpus::add) to add interesting sequences to
//!   the collection.
//!
//! [`Fuzzer`](crate::fuzzer::factory::Fuzzer) is responsible for:
//! - Using [`next`](SharedCorpus::next) to obtain the next input to execute.
//! - Using [`add`](SharedCorpus::add) to store interesting sequences
//!   discovered during execution.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;

use alloy_dyn_abi::{DynSolValue, Specifier};
use alloy_json_abi::StateMutability;
use anyhow::{Result, ensure};

pub use call::Call;
pub use extractor::{ExtractedLiterals, extract_literals};
pub use item::Item;

use crate::fuzzer::corpus::random::RandomDynSolValue;
use crate::target::Contract;

pub mod call;
pub mod extractor;
pub mod item;
pub mod random;

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

/// Inner state of [`SharedCorpus`].
pub struct SharedCorpusInner {
    pub corpus_dir: PathBuf,
    pub items: RwLock<SharedCorpusItems>,
    pub contract: Contract,
    pub max_calls_length: usize,
    pub literals: ExtractedLiterals,
}

impl std::fmt::Debug for SharedCorpusInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.items.read().vec.len();
        f.debug_struct("SharedCorpusInner")
            .field("items", &format_args!("[{len} items]"))
            .field("contract", &self.contract)
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
    pub fn new(
        dir: impl AsRef<Path>,
        contract: Contract,
        max_calls_length: usize,
        literals: ExtractedLiterals,
    ) -> Self {
        let inner = Arc::new(SharedCorpusInner {
            corpus_dir: dir.as_ref().to_path_buf(),
            items: RwLock::new(SharedCorpusItems {
                ids: HashSet::new(),
                vec: Vec::new(),
            }),
            contract,
            max_calls_length,
            literals,
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

        let dir = self.inner.corpus_dir.clone();

        if dir.exists() {
            for entry in walkdir::WalkDir::new(dir)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.extension() == Some("json".as_ref()) {
                    total_count += 1;

                    let Ok(json) = fs::read_to_string(path) else {
                        parse_failed_count += 1;
                        continue;
                    };

                    let Ok(item) = serde_json::from_str::<Item>(&json) else {
                        parse_failed_count += 1;
                        continue;
                    };

                    let all_valid = item.calls.iter().all(|call| {
                        self.inner.contract.abi.functions().any(|f| {
                            f.selector() == call.selector()
                                && f.signature() == call.function.signature()
                        })
                    });

                    if all_valid {
                        valid_count += 1;
                        let mut items = self.inner.items.write();
                        items.try_add(item);
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
        {
            let mut items = self.inner.items.write();
            if !items.try_add(item.clone()) {
                return Ok(());
            }
        }

        // Write to disk
        let path = item.path(&self.inner.corpus_dir, &self.inner.contract.artifact_id);
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
        let functions = &self.inner.contract.target_functions;
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
            Some(random::random_uint(rng, 256, &self.inner.literals))
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

    /// Get the next corpus item for execution.
    ///
    /// Returns a freshly generated item 30% of the time (or when the corpus
    /// is empty). The remaining 70% of the time returns a random existing
    /// item.
    pub fn next(&self, rng: &mut fastrand::Rng) -> Item {
        if self.is_fresh_item(rng) {
            return self.generate_item(rng);
        }
        match self.pick_item(rng) {
            Ok(item) => item,
            Err(_) => unreachable!("is_fresh_item guarantees non-empty corpus"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Barrier;

    use revm::primitives::Bytes;

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
            initcode: Bytes::new(),
        };

        let corpus = SharedCorpus::new(tmp.path(), contract, 4, ExtractedLiterals::default());

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

        let expected_path = item.path(tmp.path(), &corpus.inner.contract.artifact_id);
        assert!(
            expected_path.exists(),
            "corpus item should be written to disk at {:?}",
            expected_path
        );

        // Ensure there is exactly one file in the entire tree.
        let file_count: usize = walkdir::WalkDir::new(tmp.path())
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
            initcode: Bytes::new(),
        };

        let corpus = SharedCorpus::new(tmp.path(), contract, 8, ExtractedLiterals::default());

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
            initcode: Bytes::new(),
        };

        let corpus = SharedCorpus::new(tmp.path(), contract, 4, ExtractedLiterals::default());

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
            initcode: Bytes::new(),
        };

        let corpus = SharedCorpus::new(tmp.path(), contract, 4, ExtractedLiterals::default());
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
            initcode: Bytes::new(),
        };

        let corpus = SharedCorpus::new(tmp.path(), contract, 4, ExtractedLiterals::default());
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
            initcode: Bytes::new(),
        };

        let max_calls = 64;
        let corpus = SharedCorpus::new(
            tmp.path(),
            contract,
            max_calls,
            ExtractedLiterals::default(),
        );

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
            initcode: Bytes::new(),
        };

        let corpus = SharedCorpus::new(tmp.path(), contract, 64, ExtractedLiterals::default());

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
            initcode: Bytes::new(),
        };

        let corpus = SharedCorpus::new(tmp.path(), contract, 64, ExtractedLiterals::default());

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
            initcode: Bytes::new(),
        };

        let corpus = SharedCorpus::new(tmp.path(), contract, 64, ExtractedLiterals::default());

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
            initcode: Bytes::new(),
        };

        let corpus = SharedCorpus::new(tmp.path(), contract, 64, ExtractedLiterals::default());

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
}
