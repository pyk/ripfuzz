//! Thread-safe corpus shared across parallel fuzzer threads.
//!
//! ## Separation of concerns
//!
//! `SharedCorpus` is responsible for:
//! - Loading and validating corpus from disk.
//! - Defining [`Item`] which is convertible to/from
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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
pub use call::Call;
pub use item::Item;

use crate::target::Contract;

pub mod call;
pub mod item;

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
    pub fn new(dir: impl AsRef<Path>, contract: Contract) -> Self {
        let inner = Arc::new(SharedCorpusInner {
            corpus_dir: dir.as_ref().to_path_buf(),
            items: papaya::HashMap::new(),
            contract,
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

    /// Take a corpus item for execution.
    ///
    /// Picks a random existing item and returns a clone.
    /// If the corpus is empty, an empty item is returned.
    pub fn take(&self, rng: &mut fastrand::Rng) -> Item {
        let map = self.inner.items.pin();
        let count = map.len();
        if count > 0 {
            let items: Vec<Item> = map.values().cloned().collect();
            let idx = rng.usize(0..items.len());
            return items[idx].clone();
        }

        Item::from(Vec::new())
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

        let corpus = SharedCorpus::new(tmp.path(), contract);

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
}
