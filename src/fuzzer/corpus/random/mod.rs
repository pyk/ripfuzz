//! Random value generation helpers seeded with extracted literals.

use alloy_primitives::U256;
use fastrand::Rng;

pub use int::int;
pub use uint::uint;
pub mod int;
pub mod uint;

/// Pick a random item from a slice, or return `None` if empty.
pub fn pick_random<T: Clone>(items: &[T], rng: &mut Rng) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(items[rng.usize(0..items.len())].clone())
    }
}

/// Parse a Solidity number literal string into [`U256`].
///
/// Handles both decimal and `0x` prefixed hex literals.
pub fn parse_number_literal(val: &str) -> Option<U256> {
    let trimmed = val.trim();
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        U256::from_str_radix(&trimmed[2..], 16).ok()
    } else {
        U256::from_str_radix(trimmed, 10).ok()
    }
}
