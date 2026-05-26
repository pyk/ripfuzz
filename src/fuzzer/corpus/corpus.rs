//! Thread-safe corpus with per-contract coverage tracking.

use std::path::{Path, PathBuf};

use crate::evm::coverage::map::CoverageMap;
use crate::fuzzer::corpus::Item;

/// Inner mutable state protected by [`SharedCorpusInner`]'s lock.
#[derive(Debug)]
pub struct Corpus {
    /// Sequences loaded from disk that have not been replayed yet.
    pub pending: Vec<Item>,
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

    pub fn with_seeds(seeds: Vec<Item>) -> Self {
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
    pub fn pop_pending_item(&mut self) -> Option<Item> {
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
