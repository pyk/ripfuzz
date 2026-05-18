//! Thread-safe corpus with per-contract coverage tracking.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use alloy_primitives::B256;
use serde::{Deserialize, Serialize};

use crate::corpus::Call;

/// Rotate-left by 32 bits, matching Medusa's edge marker encoding.
pub(crate) fn edge_marker(src_pc: usize, dst_pc: usize) -> u64 {
    (src_pc as u64).rotate_left(32) ^ (dst_pc as u64)
}

/// Number of PCs for which call-depth sensitivity is tracked per contract.
pub const DEPTH_TRACKED_PCS: usize = 1_024;

/// Identifier for a contract's bytecode (keccak256 hash).
pub type ContractId = B256;

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

/// AFL-style hitcount bucket for a raw hit count.
fn afl_bucket(raw: u8) -> u8 {
    match raw {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 4,
        4..=7 => 8,
        8..=15 => 16,
        16..=31 => 32,
        32..=127 => 64,
        _ => 128,
    }
}

/// Result of merging a local coverage map into the global map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoverageUpdate {
    pub new_edges: usize,
    pub new_features: usize,
    pub new_depths: usize,
    pub new_reverts: usize,
    pub new_jump_edges: usize,
    pub new_jump_features: usize,
}

/// Coverage data for a single contract.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContractCoverage {
    /// Per-PC hitcount buckets. Length equals bytecode length.
    pub edges: Vec<u8>,
    /// Per-PC call-depth bitset (only first `DEPTH_TRACKED_PCS` PCs).
    pub depths: Vec<u64>,
    /// Per-PC revert bitset (packed, one bit per PC).
    pub reverts: Vec<u64>,
    /// Branch-direction hitcount buckets for JUMP / JUMPI edges.
    /// Key = Medusa-style edge marker; value = raw hit count.
    pub jump_edges: HashMap<u64, u8>,
}

impl ContractCoverage {
    pub fn new(bytecode_len: usize) -> Self {
        let depth_len = bytecode_len.min(DEPTH_TRACKED_PCS);
        let revert_words = bytecode_len.div_ceil(64);
        Self {
            edges: vec![0u8; bytecode_len],
            depths: vec![0u64; depth_len],
            reverts: vec![0u64; revert_words],
            jump_edges: HashMap::new(),
        }
    }
}

/// Global coverage map keyed by contract bytecode hash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageMap {
    pub contracts: HashMap<ContractId, ContractCoverage>,
}

impl CoverageMap {
    /// Merge a local coverage map into this global map.
    pub fn merge(&mut self, local: &LocalCoverage) -> CoverageUpdate {
        let mut update = CoverageUpdate::default();

        for (contract_id, local_contract) in &local.contracts {
            let global = self
                .contracts
                .entry(*contract_id)
                .or_insert_with(|| ContractCoverage::new(local_contract.edges.len()));

            // Merge edges
            for i in 0..local_contract.edges.len() {
                let local_raw = local_contract.edges[i];
                if local_raw == 0 {
                    continue;
                }
                let local_bucket = afl_bucket(local_raw);
                let hist = global.edges[i];
                if hist == 0 {
                    update.new_edges += 1;
                } else if local_bucket > afl_bucket(hist) {
                    update.new_features += 1;
                }
                global.edges[i] = global.edges[i].max(local_bucket);
            }

            // Merge depths
            let depth_len = local_contract.depths.len().min(global.depths.len());
            for i in 0..depth_len {
                let local_depth = local_contract.depths[i];
                if local_depth == 0 {
                    continue;
                }
                let prev = global.depths[i];
                if prev != local_depth {
                    let new_bits = local_depth & !prev;
                    if new_bits != 0 {
                        update.new_depths += 1;
                    }
                    global.depths[i] |= local_depth;
                }
            }

            // Merge reverts
            let revert_len = local_contract.reverts.len().min(global.reverts.len());
            for i in 0..revert_len {
                let local_rev = local_contract.reverts[i];
                if local_rev == 0 {
                    continue;
                }
                let prev = global.reverts[i];
                if prev != local_rev {
                    let new_bits = local_rev & !prev;
                    if new_bits != 0 {
                        update.new_reverts += 1;
                    }
                    global.reverts[i] |= local_rev;
                }
            }

            // Merge jump edges
            for (&marker, &local_raw) in &local_contract.jump_edges {
                if local_raw == 0 {
                    continue;
                }
                let local_bucket = afl_bucket(local_raw);
                let entry = global.jump_edges.entry(marker).or_insert(0);
                let hist = *entry;
                if hist == 0 {
                    update.new_jump_edges += 1;
                } else if local_bucket > afl_bucket(hist) {
                    update.new_jump_features += 1;
                }
                *entry = (*entry).max(local_bucket);
            }
        }

        update
    }

    /// Whether the update represents interesting new coverage.
    pub fn is_interesting(update: &CoverageUpdate) -> bool {
        update.new_edges > 0
            || update.new_features > 0
            || update.new_depths > 0
            || update.new_reverts > 0
            || update.new_jump_edges > 0
            || update.new_jump_features > 0
    }
}

/// Per-worker local coverage map keyed by contract bytecode hash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalCoverage {
    pub contracts: HashMap<ContractId, LocalContractCoverage>,
}

/// Coverage data for a single contract in a worker's local map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalContractCoverage {
    /// Per-PC raw hit counts.
    pub edges: Vec<u8>,
    /// Per-PC call-depth bitset.
    pub depths: Vec<u64>,
    /// Per-PC revert bitset.
    pub reverts: Vec<u64>,
    /// Branch-direction hitcount buckets for JUMP / JUMPI edges.
    pub jump_edges: HashMap<u64, u8>,
}

impl LocalContractCoverage {
    pub fn new(bytecode_len: usize) -> Self {
        let depth_len = bytecode_len.min(DEPTH_TRACKED_PCS);
        let revert_words = bytecode_len.div_ceil(64);
        Self {
            edges: vec![0u8; bytecode_len],
            depths: vec![0u64; depth_len],
            reverts: vec![0u64; revert_words],
            jump_edges: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.edges.fill(0);
        self.depths.fill(0);
        self.reverts.fill(0);
        self.jump_edges.clear();
    }
}

impl LocalCoverage {
    pub fn new() -> Self {
        Self {
            contracts: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        for coverage in self.contracts.values_mut() {
            coverage.clear();
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
    pub fn load(dir: impl AsRef<Path>) -> anyhow::Result<Self> {
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
    pub fn flush_to_disk(&self) -> anyhow::Result<()> {
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
