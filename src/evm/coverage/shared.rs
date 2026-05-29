//! Shared coverage map for parallel fuzzing.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use alloy_primitives::B256;
use papaya::HashMap;

use crate::evm::coverage::edge::DEPTH_TRACKED_PCS;
use crate::evm::coverage::exec::ExecutionCoverage;

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
    pub jump_edges: HashMap<u64, AtomicU8>,
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
            jump_edges: HashMap::new(),
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
/// * `jump_edges` uses `papaya::HashMap` for lock-free concurrent updates.
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
    pub fn merge(&self, local: &ExecutionCoverage) -> CoverageUpdate {
        let mut update = CoverageUpdate::default();
        let guard = self.inner.contracts.pin();

        for (contract_id, local_contract) in &local.contracts {
            let global = guard.get_or_insert(
                *contract_id,
                ContractCoverage::new(local_contract.edges.len()),
            );

            // Merge edges: only iterate over PCs that were actually hit.
            for &pc in &local_contract.hit_pcs {
                let local_raw = local_contract.edges[pc];
                let local_bucket = afl_bucket(local_raw);
                let prev = global.edges[pc].fetch_max(local_bucket, Ordering::Relaxed);
                if prev == 0 {
                    update.new_edges += 1;
                } else if local_bucket > afl_bucket(prev) {
                    update.new_features += 1;
                }
            }

            // Merge depths: only iterate over PCs that recorded a depth.
            for &pc in &local_contract.hit_depths {
                let local_depth = local_contract.depths[pc];
                let prev = global.depths[pc].fetch_or(local_depth, Ordering::Relaxed);
                let new_bits = local_depth & !prev;
                if new_bits != 0 {
                    update.new_depths += 1;
                }
            }

            // Merge reverts: only iterate over word indices that were actually hit.
            for &i in &local_contract.hit_reverts {
                if i >= global.reverts.len() {
                    continue;
                }
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

            // Merge jump edges: lock-free get-or-insert, then atomic fetch_max.
            let jump_guard = global.jump_edges.pin();
            for (&marker, &local_raw) in &local_contract.jump_edges {
                if local_raw == 0 {
                    continue;
                }
                let local_bucket = afl_bucket(local_raw);
                let entry = jump_guard.get_or_insert_with(marker, || AtomicU8::new(0));
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
}

impl CoverageUpdate {
    /// Whether this update represents interesting new coverage.
    pub fn is_interesting(&self) -> bool {
        self.new_edges > 0
            || self.new_features > 0
            || self.new_depths > 0
            || self.new_reverts > 0
            || self.new_jump_edges > 0
            || self.new_jump_features > 0
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

    use crate::evm::coverage::CoverageUpdate;
    use crate::evm::coverage::exec::{ExecutionContractCoverage, ExecutionCoverage};

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

        // Build a local coverage with one contract and one signal for every
        // merge loop: edges, depths, reverts, and jump edges.
        let mut local = ExecutionCoverage::new();
        let contract_id = B256::ZERO;
        let mut contract = ExecutionContractCoverage::new(1024);

        // Edges
        contract.edges[10] = 1;
        contract.hit_pcs.push(10);

        // Depths
        contract.depths[20] = 1 << 3;
        contract.hit_depths.push(20);

        // Reverts
        contract.reverts[5] = 1 << 10;
        contract.hit_reverts.push(5);

        // Jump edges
        contract.jump_edges.insert(0x1234, 1);

        local.contracts.insert(contract_id, contract);

        let mut total = CoverageUpdate::default();
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
                total.new_edges += update.new_edges;
                total.new_features += update.new_features;
                total.new_depths += update.new_depths;
                total.new_reverts += update.new_reverts;
                total.new_jump_edges += update.new_jump_edges;
                total.new_jump_features += update.new_jump_features;
                if update.is_interesting() {
                    interesting_count += 1;
                }
            }
        });

        // Each signal was discovered exactly once across all threads.
        assert_eq!(
            total.new_edges, 1,
            "exactly one thread discovered the new edge"
        );
        assert_eq!(
            total.new_depths, 1,
            "exactly one thread discovered the new depth"
        );
        assert_eq!(
            total.new_reverts, 1,
            "exactly one thread discovered the new revert"
        );
        assert_eq!(
            total.new_jump_edges, 1,
            "exactly one thread discovered the new jump edge"
        );
        assert!(
            interesting_count >= 1,
            "at least one thread should find identical coverage interesting"
        );
    }

    /// Sixteen threads, each with a unique local coverage signal.
    /// Every thread should discover something new, so all 16 are interesting.
    #[test]
    fn sixteen_threads_all_unique() {
        let shared = SharedCoverage::new();
        let barrier = std::sync::Barrier::new(16);

        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for i in 0..16u64 {
                let shared = shared.clone();
                let barrier_ref = &barrier;
                handles.push(s.spawn(move || {
                    let mut local = ExecutionCoverage::new();
                    let mut contract = ExecutionContractCoverage::new(1024);
                    let contract_id = B256::ZERO;

                    // Unique edge per thread.
                    let pc = i as usize;
                    contract.edges[pc] = 1;
                    contract.hit_pcs.push(pc);

                    // Unique depth per thread.
                    let depth_pc = i as usize;
                    contract.depths[depth_pc] = 1 << (i % 64);
                    contract.hit_depths.push(depth_pc);

                    // Unique revert per thread.
                    let rev_word = i as usize;
                    let rev_bit = (i % 64) as u64;
                    contract.reverts[rev_word] = 1 << rev_bit;
                    contract.hit_reverts.push(rev_word);

                    // Unique jump edge per thread.
                    contract.jump_edges.insert(i, 1);

                    local.contracts.insert(contract_id, contract);
                    barrier_ref.wait();
                    let update = shared.merge(&local);
                    assert!(
                        update.is_interesting(),
                        "thread {i} should be interesting with unique coverage"
                    );
                    update
                }));
            }

            let mut total = CoverageUpdate::default();
            for handle in handles {
                let update = handle.join().unwrap();
                total.new_edges += update.new_edges;
                total.new_features += update.new_features;
                total.new_depths += update.new_depths;
                total.new_reverts += update.new_reverts;
                total.new_jump_edges += update.new_jump_edges;
                total.new_jump_features += update.new_jump_features;
            }

            // 16 unique signals for each type, each discovered exactly once.
            assert_eq!(
                total.new_edges, 16,
                "all 16 unique edges should be discovered"
            );
            assert_eq!(
                total.new_depths, 16,
                "all 16 unique depths should be discovered"
            );
            assert_eq!(
                total.new_reverts, 16,
                "all 16 unique reverts should be discovered"
            );
            assert_eq!(
                total.new_jump_edges, 16,
                "all 16 unique jump edges should be discovered"
            );
        });
    }
}
