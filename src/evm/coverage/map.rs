//! Coverage maps: global, local, per-contract, and merge results.

use std::collections::HashMap;

use alloy_primitives::B256;

use crate::evm::coverage::edge::DEPTH_TRACKED_PCS;

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
    pub contracts: HashMap<B256, ContractCoverage>,
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

    /// Total number of unique coverage hits (edges + jump edges) across all contracts.
    pub fn hit_count(&self) -> usize {
        let edge_hits: usize = self
            .contracts
            .values()
            .map(|c| c.edges.iter().filter(|&&e| e != 0).count())
            .sum();
        let jump_hits: usize = self.contracts.values().map(|c| c.jump_edges.len()).sum();
        edge_hits + jump_hits
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

/// Per-fuzzer local coverage map keyed by contract bytecode hash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalCoverage {
    pub contracts: HashMap<B256, LocalContractCoverage>,
}

/// Coverage data for a single contract in a fuzzer's local map.
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
