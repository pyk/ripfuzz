//! Random unsigned integer generation, seeded with extracted number literals.

use alloy_primitives::U256;
use fastrand::Rng;

use crate::fuzzer::corpus::ExtractedLiterals;
use crate::fuzzer::corpus::random::{parse_number_literal, pick_random};

/// Probability (0-100) of entering the literal phase before edge cases or
/// random fallback.
const LITERAL_BIAS: u32 = 40;

/// Generate a random unsigned integer of the given bit width.
///
/// With `LITERAL_BIAS` % probability, a random literal from
/// `literals.numbers` is chosen and parsed. If it fits the type it is
/// returned immediately. If the dice roll misses, or the chosen literal is
/// out of range, the generator falls through.
///
/// The second phase picks edge-case values (`0`, `1`, `max`, `max-1`,
/// `max-2`, `max-3`) with 50 % probability. Otherwise a uniformly random
/// value masked to the correct bit width is returned.
pub fn uint(bits: usize, literals: &ExtractedLiterals, rng: &mut Rng) -> U256 {
    let max = max_for_bits(bits);

    // 1. Try a literal that fits (only some of the time).
    if !literals.numbers.is_empty()
        && rng.u32(0..100) < LITERAL_BIAS
        && let Some(val) = pick_random(&literals.numbers, rng)
        && let Some(u) = parse_number_literal(&val)
        && u <= max
    {
        return u;
    }

    // 2. Edge cases: 0, 1, max, max-1, max-2, max-3.
    if rng.bool() {
        let edge = match rng.u32(0..6) {
            0 => U256::ZERO,
            1 => U256::from(1),
            2 => max,
            3 => max.saturating_sub(U256::from(1)),
            4 => max.saturating_sub(U256::from(2)),
            _ => max.saturating_sub(U256::from(3)),
        };
        return edge;
    }

    // 3. Fallback: uniformly random value masked to the bit width.
    let low = rng.u128(..);
    let high = rng.u128(..);
    let raw = U256::from(low) | (U256::from(high) << 128);
    if bits == 256 { raw } else { raw & max }
}

/// Compute the maximum value for an unsigned integer of `bits` width.
fn max_for_bits(bits: usize) -> U256 {
    if bits == 0 {
        U256::ZERO
    } else if bits >= 256 {
        U256::MAX
    } else {
        (U256::from(1) << bits) - U256::from(1)
    }
}
