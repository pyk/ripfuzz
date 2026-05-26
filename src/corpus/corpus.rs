//! Thread-safe corpus with per-contract coverage tracking.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use serde::{Deserialize, Serialize};

use crate::corpus::Call;
use crate::evm::coverage::map::{CoverageMap, LocalCoverage};

/// A single item in the fuzzing corpus.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CorpusItem {
    pub calls: Vec<Call>,
    pub weight: u64,
    #[serde(default)]
    pub total_mutations: u64,
    #[serde(default)]
    pub new_finds_produced: u64,
}

impl CorpusItem {
    pub fn new(calls: Vec<Call>) -> Self {
        Self {
            calls,
            weight: 1,
            total_mutations: 0,
            new_finds_produced: 0,
        }
    }
}

/// Thread-safe corpus with coverage tracking.
#[derive(Debug)]
pub struct Corpus {
    /// Coverage-increasing sequences available for mutation.
    pub items: Vec<CorpusItem>,
    /// Crash sequences discovered during the campaign.
    pub failures: Vec<CorpusItem>,
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
            items: Vec::new(),
            failures: Vec::new(),
            pending: Vec::new(),
            coverage: CoverageMap::default(),
            storage_dir: None,
        }
    }

    pub fn with_seeds(seeds: Vec<CorpusItem>) -> Self {
        Self {
            items: Vec::new(),
            failures: Vec::new(),
            pending: seeds,
            coverage: CoverageMap::default(),
            storage_dir: None,
        }
    }

    /// Build a corpus from an optional on-disk directory.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let mut pending = Vec::new();
        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension() == Some("json".as_ref()) {
                    let json = fs::read_to_string(&path)?;
                    let item: CorpusItem = serde_json::from_str(&json)?;
                    pending.push(item);
                }
            }
        }
        Ok(Self {
            items: Vec::new(),
            failures: Vec::new(),
            pending,
            coverage: CoverageMap::default(),
            storage_dir: Some(dir.to_path_buf()),
        })
    }

    /// Set the directory used for persistent corpus storage.
    pub fn set_storage_dir(&mut self, dir: impl AsRef<Path>) {
        self.storage_dir = Some(dir.as_ref().to_path_buf());
    }

    /// Weighted random pick for mutation target.
    pub fn random_item_for_mutation(&self, rng: &mut fastrand::Rng) -> Option<CorpusItem> {
        if self.items.is_empty() {
            return None;
        }
        let total_weight: u64 = self
            .items
            .iter()
            .map(|i| 1u64 + i.new_finds_produced.saturating_mul(10))
            .sum();
        let target = rng.u64(0..total_weight);
        let mut cumulative = 0u64;
        let mut selected = 0usize;
        for (i, item) in self.items.iter().enumerate() {
            cumulative += 1u64 + item.new_finds_produced.saturating_mul(10);
            if cumulative > target {
                selected = i;
                break;
            }
        }
        self.items.get(selected).cloned()
    }

    /// Weighted random pick that also returns the item index.
    pub fn random_item_for_mutation_with_index(
        &self,
        rng: &mut fastrand::Rng,
    ) -> Option<(usize, CorpusItem)> {
        if self.items.is_empty() {
            return None;
        }
        let total_weight: u64 = self
            .items
            .iter()
            .map(|i| 1u64 + i.new_finds_produced.saturating_mul(10))
            .sum();
        let target = rng.u64(0..total_weight);
        let mut cumulative = 0u64;
        let mut selected = 0usize;
        for (i, item) in self.items.iter().enumerate() {
            cumulative += 1u64 + item.new_finds_produced.saturating_mul(10);
            if cumulative > target {
                selected = i;
                break;
            }
        }
        self.items
            .get(selected)
            .cloned()
            .map(|item| (selected, item))
    }

    /// Whether the corpus has any entries for mutation.
    pub fn has_entries(&self) -> bool {
        !self.items.is_empty()
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

    /// Replace the global coverage map.
    pub fn set_coverage(&mut self, coverage: CoverageMap) {
        self.coverage = coverage;
    }

    /// Merge local coverage and add item if interesting.
    pub fn check_and_update_coverage(&mut self, local: &LocalCoverage, item: &CorpusItem) -> bool {
        let update = self.coverage.merge(local);
        if CoverageMap::is_interesting(&update) {
            let already_present = self.items.iter().any(|i| i.calls == item.calls)
                || self.failures.iter().any(|i| i.calls == item.calls)
                || self.pending.iter().any(|i| i.calls == item.calls);
            if !already_present {
                self.items.push(item.clone());
                return true;
            }
        }
        false
    }

    /// Add a corpus item for mutation if its call sequence is not already present.
    pub fn add_item_for_mutation(&mut self, item: &CorpusItem) -> bool {
        let already_present = self.items.iter().any(|i| i.calls == item.calls)
            || self.failures.iter().any(|i| i.calls == item.calls)
            || self.pending.iter().any(|i| i.calls == item.calls);
        if !already_present {
            self.items.push(item.clone());
            return true;
        }
        false
    }

    /// Add a failure item (not used for mutation).
    pub fn add_failure(&mut self, item: CorpusItem) {
        let already_present = self.items.iter().any(|i| i.calls == item.calls)
            || self.failures.iter().any(|i| i.calls == item.calls)
            || self.pending.iter().any(|i| i.calls == item.calls);
        if !already_present {
            self.failures.push(item);
        }
    }

    /// Persist all corpus items and failures to disk.
    pub fn flush_to_disk(&self) -> Result<()> {
        let Some(dir) = &self.storage_dir else {
            return Ok(());
        };
        fs::create_dir_all(dir)?;

        for item in &self.items {
            let name = format!("{}.json", uuid::Uuid::new_v4());
            let path = dir.join(&name);
            let json = serde_json::to_string_pretty(item)?;
            fs::write(&path, json)?;
        }

        for item in &self.failures {
            let name = format!("failure-{}.json", uuid::Uuid::new_v4());
            let path = dir.join(&name);
            let json = serde_json::to_string_pretty(item)?;
            fs::write(&path, json)?;
        }

        Ok(())
    }
}
