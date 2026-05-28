//! Local coverage maps for a single fuzzer thread.

use std::collections::HashMap;

use alloy_primitives::B256;

use crate::evm::coverage::edge::DEPTH_TRACKED_PCS;

/// Per-fuzzer local coverage map keyed by contract bytecode hash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionCoverage {
    pub contracts: HashMap<B256, ExecutionContractCoverage>,
}

impl ExecutionCoverage {
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

/// Coverage data for a single contract in a fuzzer's local map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionContractCoverage {
    /// Per-PC raw hit counts.
    pub edges: Vec<u8>,
    /// Per-PC call-depth bitset.
    pub depths: Vec<u64>,
    /// Per-PC revert bitset.
    pub reverts: Vec<u64>,
    /// Branch-direction hitcount buckets for JUMP / JUMPI edges.
    pub jump_edges: HashMap<u64, u8>,
    /// Sparse list of PCs that were hit during this execution.
    /// Used to avoid iterating over the entire `edges` array during merge.
    pub hit_pcs: Vec<usize>,
    /// Sparse list of PCs that recorded a depth during this execution.
    /// Used to avoid iterating over the entire `depths` array during merge.
    pub hit_depths: Vec<usize>,
    /// Sparse list of word indices that recorded a revert during this execution.
    /// Used to avoid iterating over the entire `reverts` array during merge.
    pub hit_reverts: Vec<usize>,
}

impl ExecutionContractCoverage {
    pub fn new(bytecode_len: usize) -> Self {
        let depth_len = bytecode_len.min(DEPTH_TRACKED_PCS);
        let revert_words = bytecode_len.div_ceil(64);
        Self {
            edges: vec![0u8; bytecode_len],
            depths: vec![0u64; depth_len],
            reverts: vec![0u64; revert_words],
            jump_edges: HashMap::new(),
            hit_pcs: Vec::new(),
            hit_depths: Vec::new(),
            hit_reverts: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.edges.fill(0);
        self.depths.fill(0);
        self.reverts.fill(0);
        self.jump_edges.clear();
        self.hit_pcs.clear();
        self.hit_depths.clear();
        self.hit_reverts.clear();
    }
}
