//! Edge encoding primitives for coverage tracking.

/// Number of PCs for which call-depth sensitivity is tracked per contract.
pub const DEPTH_TRACKED_PCS: usize = 1_024;

/// Rotate-left by 32 bits, matching Medusa's edge marker encoding.
pub fn edge_marker(src_pc: usize, dst_pc: usize) -> u64 {
    (src_pc as u64).rotate_left(32) ^ (dst_pc as u64)
}
