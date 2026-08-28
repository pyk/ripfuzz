//! Edge encoding primitives for coverage tracking.

use alloy_primitives::Address;

/// Number of PCs for which call-depth sensitivity is tracked per contract.
pub const DEPTH_TRACKED_PCS: usize = 1_024;

/// Rotate-left by 32 bits, matching Medusa's edge marker encoding.
pub fn edge_marker(src_pc: usize, dst_pc: usize) -> u64 {
    (src_pc as u64).rotate_left(32) ^ (dst_pc as u64)
}

/// Marker for a CALL edge `(caller_pc, callee_address)`.
///
/// Unlike jump edges that record control flow inside one contract, a call
/// edge records that the caller invoked a specific callee at a given PC.
/// Two pools with identical bytecode but different addresses therefore
/// produce distinct markers even though the caller executes the same PCs.
/// The marker mixes the caller PC with the callee address and sets bit 63
/// so it never collides with [`edge_marker`] values, which have bit 63
/// clear for realistic bytecode sizes.
pub fn call_edge_marker(caller_pc: usize, callee: Address) -> u64 {
    let addr = callee.as_slice();
    let mut low_bytes = [0u8; 8];
    low_bytes.copy_from_slice(&addr[12..20]);
    let low = u64::from_be_bytes(low_bytes);
    let mut high: u64 = 0;
    for &b in &addr[0..12] {
        high = high.wrapping_mul(131).wrapping_add(b as u64);
    }
    let pc = caller_pc as u64;
    let mut marker = pc.rotate_left(32) ^ low;
    marker ^= high.rotate_left(16);
    marker | (1u64 << 63)
}
