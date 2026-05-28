//! Shared coverage map for parallel fuzzing.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use alloy_primitives::B256;
use dashmap::DashMap;
use papaya::HashMap;

use crate::evm::coverage::edge::DEPTH_TRACKED_PCS;
use crate::evm::coverage::local::LocalCoverage;

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

/// Coverage data for a single contract in the shared global map.
///
/// Uses atomic types so that multiple fuzzer threads can merge concurrently
/// without blocking on a per-contract mutex.
pub struct ContractCoverage {
    /// Per-PC hitcount buckets. Length equals bytecode length.
    pub edges: Vec<AtomicU8>,
    /// Per-PC call-depth bitset (only first `DEPTH_TRACKED_PCS` PCs).
    pub depths: Vec<AtomicU64>,
    /// Per-PC revert bitset (packed, one bit per PC).
    pub reverts: Vec<AtomicU64>,
    /// Branch-direction hitcount buckets for JUMP / JUMPI edges.
    /// Key = Medusa-style edge marker; value = raw hit count.
    pub jump_edges: DashMap<u64, AtomicU8>,
}

impl std::fmt::Debug for ContractCoverage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContractCoverage")
            .field("edges_len", &self.edges.len())
            .field("depths_len", &self.depths.len())
            .field("reverts_len", &self.reverts.len())
            .field("jump_edges", &self.jump_edges.len())
            .finish()
    }
}

impl ContractCoverage {
    pub fn new(bytecode_len: usize) -> Self {
        let depth_len = bytecode_len.min(DEPTH_TRACKED_PCS);
        let revert_words = bytecode_len.div_ceil(64);
        Self {
            edges: (0..bytecode_len).map(|_| AtomicU8::new(0)).collect(),
            depths: (0..depth_len).map(|_| AtomicU64::new(0)).collect(),
            reverts: (0..revert_words).map(|_| AtomicU64::new(0)).collect(),
            jump_edges: DashMap::new(),
        }
    }
}

/// Global coverage map keyed by contract bytecode hash.
///
/// Designed for fast parallel fuzzing:
///
/// * Outer contract map is a lock-free `papaya::HashMap` so threads never
///   block to look up a contract.
/// * Inner per-contract arrays use atomic `fetch_max` / `fetch_or` so
///   multiple threads can update the same contract concurrently without
///   a per-contract mutex.
/// * `jump_edges` uses `DashMap` for sharded concurrent updates.
///
/// Cloning is cheap (shares the same inner state).
#[derive(Clone, Debug)]
pub struct SharedCoverage {
    inner: Arc<SharedCoverageInner>,
}

#[derive(Debug)]
pub struct SharedCoverageInner {
    contracts: HashMap<B256, ContractCoverage>,
}

impl SharedCoverage {
    /// Create a new empty shared coverage map.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SharedCoverageInner {
                contracts: HashMap::new(),
            }),
        }
    }

    /// Merge a local coverage map into this global map.
    ///
    /// Lock-free for the outer map; atomic operations for the inner arrays.
    pub fn merge(&self, local: &LocalCoverage) -> CoverageUpdate {
        let mut update = CoverageUpdate::default();
        let guard = self.inner.contracts.pin();

        for (contract_id, local_contract) in &local.contracts {
            let global = guard.get_or_insert(
                *contract_id,
                ContractCoverage::new(local_contract.edges.len()),
            );

            // Merge edges
            for i in 0..local_contract.edges.len() {
                let local_raw = local_contract.edges[i];
                if local_raw == 0 {
                    continue;
                }
                let local_bucket = afl_bucket(local_raw);
                let prev = global.edges[i].fetch_max(local_bucket, Ordering::Relaxed);
                if prev == 0 {
                    update.new_edges += 1;
                } else if local_bucket > afl_bucket(prev) {
                    update.new_features += 1;
                }
            }

            // Merge depths
            let depth_len = local_contract.depths.len().min(global.depths.len());
            for i in 0..depth_len {
                let local_depth = local_contract.depths[i];
                if local_depth == 0 {
                    continue;
                }
                let prev = global.depths[i].fetch_or(local_depth, Ordering::Relaxed);
                let new_bits = local_depth & !prev;
                if new_bits != 0 {
                    update.new_depths += 1;
                }
            }

            // Merge reverts
            let revert_len = local_contract.reverts.len().min(global.reverts.len());
            for i in 0..revert_len {
                let local_rev = local_contract.reverts[i];
                if local_rev == 0 {
                    continue;
                }
                let prev = global.reverts[i].fetch_or(local_rev, Ordering::Relaxed);
                let new_bits = local_rev & !prev;
                if new_bits != 0 {
                    update.new_reverts += 1;
                }
            }

            // Merge jump edges
            for (&marker, &local_raw) in &local_contract.jump_edges {
                if local_raw == 0 {
                    continue;
                }
                let local_bucket = afl_bucket(local_raw);
                let entry = global.jump_edges.entry(marker).or_insert(AtomicU8::new(0));
                let prev = entry.fetch_max(local_bucket, Ordering::Relaxed);
                if prev == 0 {
                    update.new_jump_edges += 1;
                } else if local_bucket > afl_bucket(prev) {
                    update.new_jump_features += 1;
                }
            }
        }

        update
    }

    /// Total number of unique coverage hits (edges + jump edges) across all contracts.
    pub fn hit_count(&self) -> usize {
        let guard = self.inner.contracts.pin();
        let edge_hits: usize = guard
            .iter()
            .map(|(_, c)| {
                c.edges
                    .iter()
                    .filter(|e| e.load(Ordering::Relaxed) != 0)
                    .count()
            })
            .sum();
        let jump_hits: usize = guard.iter().map(|(_, c)| c.jump_edges.len()).sum();
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

impl Default for SharedCoverage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::B256;

    use crate::evm::coverage::local::{LocalContractCoverage, LocalCoverage};

    use super::SharedCoverage;

    /// 16 threads with identical local coverage. Only one thread should see its
    /// coverage as interesting; the other 15 must see it as not interesting.
    ///
    /// A `Barrier` ensures all threads start their merge at the same time,
    /// maximizing contention on the shared atomic arrays.
    #[test]
    fn sixteen_threads_one_winner() {
        let shared = SharedCoverage::new();
        let barrier = std::sync::Barrier::new(16);

        // Build a local coverage with one contract and one hit edge.
        let mut local = LocalCoverage::new();
        let contract_id = B256::ZERO;
        let mut contract = LocalContractCoverage::new(1024);
        contract.edges[10] = 1;
        local.contracts.insert(contract_id, contract);

        let mut interesting_count = 0;
        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for _ in 0..16 {
                let local = local.clone();
                let shared = shared.clone();
                let barrier_ref = &barrier;
                handles.push(s.spawn(move || {
                    barrier_ref.wait();
                    shared.merge(&local)
                }));
            }

            for handle in handles {
                let update = handle.join().unwrap();
                if SharedCoverage::is_interesting(&update) {
                    interesting_count += 1;
                }
            }
        });

        assert_eq!(
            interesting_count, 1,
            "exactly one thread should find identical coverage interesting"
        );
    }
}
