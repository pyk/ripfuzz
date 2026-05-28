//! Local coverage maps for a single fuzzer thread.

use std::collections::HashMap;

use alloy_primitives::B256;

use crate::evm::coverage::edge::DEPTH_TRACKED_PCS;

/// Per-fuzzer local coverage map keyed by contract bytecode hash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalCoverage {
    pub contracts: HashMap<B256, LocalContractCoverage>,
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
