//! Thread-safe corpus with per-contract coverage tracking.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::evm::coverage::map::CoverageMap;
use crate::fuzzer::corpus::Call;

/// A single item in the fuzzing corpus.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorpusItem {
    pub calls: Vec<Call>,
    pub weight: u64,
    #[serde(default)]
    pub total_mutations: u64,
    #[serde(default)]
    pub new_finds_produced: u64,
    #[serde(skip, default)]
    pub(crate) is_replay: bool,
}

impl CorpusItem {
    pub fn new(calls: Vec<Call>) -> Self {
        Self {
            calls,
            weight: 1,
            total_mutations: 0,
            new_finds_produced: 0,
            is_replay: false,
        }
    }

    /// Unique identifier derived from the call sequence.
    pub fn id(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.calls.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// On-disk path for this corpus item.
    pub fn path(
        &self,
        corpus_dir: impl AsRef<Path>,
        artifact_id: &crate::foundry::ArtifactId,
    ) -> PathBuf {
        corpus_dir
            .as_ref()
            .join(&artifact_id.path)
            .join(&artifact_id.name)
            .join(format!("{}.json", self.id()))
    }

    /// Convert this corpus item into an [`ExecInput`] for the given caller
    /// and target address.
    pub fn into_exec_input(
        self,
        caller: alloy_primitives::Address,
        target: alloy_primitives::Address,
    ) -> crate::evm::chain::ExecInput {
        use crate::evm::chain::{ExecInput, Transaction};
        use revm::primitives::Bytes;
        ExecInput::new(
            self.calls
                .into_iter()
                .map(|call| {
                    Transaction::new(target)
                        .caller(caller)
                        .calldata(Bytes::from(call.encode()))
                })
                .collect(),
        )
    }
}

impl From<Vec<Call>> for CorpusItem {
    fn from(calls: Vec<Call>) -> Self {
        Self::new(calls)
    }
}

/// Inner mutable state protected by [`SharedCorpusInner`]'s lock.
#[derive(Debug)]
pub struct Corpus {
    /// Sequences loaded from disk that have not been replayed yet.
    pub pending: Vec<CorpusItem>,
    /// Global coverage map.
    coverage: CoverageMap,
    /// Directory for persistent storage, if any.
    storage_dir: Option<PathBuf>,
}

impl Default for Corpus {
    fn default() -> Self {
        Self::new()
    }
}

impl Corpus {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            coverage: CoverageMap::default(),
            storage_dir: None,
        }
    }

    pub fn with_seeds(seeds: Vec<CorpusItem>) -> Self {
        Self {
            pending: seeds,
            coverage: CoverageMap::default(),
            storage_dir: None,
        }
    }

    /// Access the storage directory, if set.
    pub fn storage_dir(&self) -> &Option<PathBuf> {
        &self.storage_dir
    }

    /// Set the directory used for persistent corpus storage.
    pub fn set_storage_dir(&mut self, dir: impl AsRef<Path>) {
        self.storage_dir = Some(dir.as_ref().to_path_buf());
    }

    /// Pop a pending item for replay.
    pub fn pop_pending_item(&mut self) -> Option<CorpusItem> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.remove(0))
        }
    }

    /// Access the global coverage map.
    pub fn coverage(&self) -> &CoverageMap {
        &self.coverage
    }

    /// Mutable access to the global coverage map.
    pub fn coverage_mut(&mut self) -> &mut CoverageMap {
        &mut self.coverage
    }

    /// Replace the global coverage map.
    pub fn set_coverage(&mut self, coverage: CoverageMap) {
        self.coverage = coverage;
    }
}
