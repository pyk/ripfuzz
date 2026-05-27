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

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use alloy_dyn_abi::{DynSolType, DynSolValue, Specifier};
use alloy_primitives::{Address, FixedBytes};
use anyhow::Result;

pub use call::Call;
pub use extractor::{ExtractedLiterals, extract_literals};
pub use item::Item;

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

/// Inner state of [`SharedCorpus`].
pub struct SharedCorpusInner {
    pub corpus_dir: PathBuf,
    pub items: papaya::HashMap<String, Item>,
    pub contract: Contract,
    pub max_calls_length: usize,
    pub literals: ExtractedLiterals,
}

impl std::fmt::Debug for SharedCorpusInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedCorpusInner")
            .field("items", &format_args!("[{} items]", self.items.pin().len()))
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
            items: papaya::HashMap::new(),
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
                        let id = item.id();
                        let _ = self.inner.items.pin().insert(id, item);
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

    /// Take the next corpus item for execution.
    ///
    /// Picks a random existing item and returns a clone.
    /// If the corpus is empty, a freshly generated random sequence is returned.
    pub fn next(&self, rng: &mut fastrand::Rng) -> Item {
        let map = self.inner.items.pin();
        let count = map.len();
        if count > 0 {
            let items: Vec<Item> = map.values().cloned().collect();
            let idx = rng.usize(0..items.len());
            return items[idx].clone();
        }

        Item::from(self.generate_random_sequence(rng))
    }

    /// Add a corpus item to the collection.
    ///
    /// Deduplicates by [`Item::id`]. Returns `Ok(())` whether the item
    /// already exists or was newly inserted. Only the thread that wins the
    /// atomic insert performs disk I/O, so the same item is never written
    /// twice even under concurrent calls.
    pub fn add(&self, item: Item) -> Result<()> {
        let id = item.id();

        // Only the first thread to successfully insert runs the closure
        // and reaches the disk-write code below. All other racing threads
        // receive `Err` from `try_insert_with` and return early.
        {
            let map = self.inner.items.pin();
            if map.try_insert_with(id, || item.clone()).is_err() {
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
        let item_count = self.inner.items.pin().len();
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
        if self.inner.items.is_empty() {
            return true;
        }
        rng.f32() < 0.30
    }
}

impl SharedCorpus {
    fn generate_random_sequence(&self, rng: &mut fastrand::Rng) -> Vec<Call> {
        let functions = &self.inner.contract.target_functions;
        let max_len = self.inner.max_calls_length.max(1);
        let len = rng.usize(1..=max_len);
        let mut calls = Vec::with_capacity(len);

        for _ in 0..len {
            if functions.is_empty() {
                break;
            }
            let idx = rng.usize(0..functions.len());
            let func = &functions[idx];
            let values: Vec<DynSolValue> = func
                .inputs
                .iter()
                .filter_map(|p| p.resolve().ok())
                .map(|ty| random_dyn_value(&ty, rng, &self.inner.literals))
                .collect();
            let call = Call {
                function: {
                    // checkrs: allow(clone_in_loops)
                    func.clone()
                },
                args: DynSolValue::Tuple(values),
                ..Default::default()
            };
            calls.push(call);
        }
        calls
    }
}

fn random_dyn_value(
    ty: &DynSolType,
    rng: &mut fastrand::Rng,
    literals: &ExtractedLiterals,
) -> DynSolValue {
    match ty {
        DynSolType::Bool => {
            if let Some(val) = random::pick_random(&literals.bool, rng) {
                return DynSolValue::Bool(val);
            }
            DynSolValue::Bool(rng.bool())
        }
        DynSolType::Uint(sz) => DynSolValue::Uint(random::uint(*sz, literals, rng), *sz),
        DynSolType::Int(sz) => DynSolValue::Int(random::int(*sz, literals, rng), *sz),
        DynSolType::FixedBytes(sz) => {
            if let Some(bucket) = literals.fixed_bytes.get(sz)
                && let Some(val) = random::pick_random(bucket, rng)
            {
                return DynSolValue::FixedBytes(val, *sz);
            }
            let mut word = [0u8; 32];
            rng.fill(&mut word);
            DynSolValue::FixedBytes(FixedBytes::from(word), *sz)
        }
        DynSolType::Address => {
            if let Some(val) = random::pick_random(&literals.address, rng) {
                return DynSolValue::Address(val);
            }
            let mut bytes = [0u8; 20];
            rng.fill(&mut bytes);
            DynSolValue::Address(Address::from_slice(&bytes))
        }
        DynSolType::Bytes => {
            if let Some(val) = random::pick_random(&literals.bytes, rng) {
                return DynSolValue::Bytes(val.to_vec());
            }
            let len = rng.usize(0..=64);
            let mut bytes = vec![0u8; len];
            rng.fill(&mut bytes);
            DynSolValue::Bytes(bytes)
        }
        DynSolType::String => {
            if let Some(val) = random::pick_random(&literals.string, rng) {
                return DynSolValue::String(val);
            }
            let len = rng.usize(0..=32);
            let s: String = (0..len).map(|_| rng.alphabetic()).collect();
            DynSolValue::String(s)
        }
        DynSolType::Function => {
            let mut bytes = [0u8; 24];
            rng.fill(&mut bytes);
            DynSolValue::Function(alloy_primitives::Function::from_slice(&bytes))
        }
        DynSolType::Array(inner) => {
            let len = rng.usize(0..=4);
            let arr: Vec<DynSolValue> = (0..len)
                .map(|_| random_dyn_value(inner, rng, literals))
                .collect();
            DynSolValue::Array(arr)
        }
        DynSolType::FixedArray(inner, len) => {
            let arr: Vec<DynSolValue> = (0..*len)
                .map(|_| random_dyn_value(inner, rng, literals))
                .collect();
            DynSolValue::FixedArray(arr)
        }
        DynSolType::Tuple(types) => {
            let values: Vec<DynSolValue> = types
                .iter()
                .map(|t| random_dyn_value(t, rng, literals))
                .collect();
            DynSolValue::Tuple(values)
        }
    }
}

#[cfg(test)]
mod tests {
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
                corpus.add(item)
            }));
        }

        for handle in handles {
            handle.join().unwrap().expect("add should succeed");
        }

        assert_eq!(
            corpus.inner.items.pin().len(),
            1,
            "exactly one item should be in the map"
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
        corpus.add(item).unwrap();

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
}
