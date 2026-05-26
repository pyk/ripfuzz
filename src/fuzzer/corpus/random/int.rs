//! Random signed integer generation, seeded with extracted number literals.

use alloy_primitives::{I256, U256};
use fastrand::Rng;

use crate::fuzzer::corpus::ExtractedLiterals;
use crate::fuzzer::corpus::random::{parse_number_literal, pick_random};

/// Probability (0-100) of entering the literal phase before edge cases or
/// random fallback.
const LITERAL_BIAS: u32 = 40;

/// Generate a random signed integer of the given bit width.
///
/// With `LITERAL_BIAS` % probability, a random literal from
/// `literals.numbers` is chosen and parsed. If it fits within the positive
/// half of the type's range it is returned immediately. If the dice roll
/// misses, or the chosen literal is out of range, the generator falls
/// through.
///
/// The second phase picks edge-case values (`min`, `min+1`, `-1`, `0`, `1`,
/// `max-1`, `max`) with 50 % probability. Otherwise a uniformly random
/// value in the full signed range is returned.
pub fn int(bits: usize, literals: &ExtractedLiterals, rng: &mut Rng) -> I256 {
    let max_positive = max_positive_for_bits(bits);

    // 1. Try a literal that fits in the positive range (only some of the time).
    if !literals.numbers.is_empty()
        && rng.u32(0..100) < LITERAL_BIAS
        && let Some(val) = pick_random(&literals.numbers, rng)
        && let Some(u) = parse_number_literal(&val)
        && u <= max_positive
    {
        return I256::try_from(u).unwrap_or(I256::ZERO);
    }

    // 2. Edge cases: min, min+1, -1, 0, 1, max-1, max.
    if rng.bool() {
        let raw = match rng.u32(0..7) {
            0 => sign_bit(bits),                 // min
            1 => sign_bit(bits) + U256::from(1), // min + 1
            2 => mask(bits),                     // -1 (all lower bits set)
            3 => U256::ZERO,                     // 0
            4 => U256::from(1),                  // 1
            5 => max_positive - U256::from(1),   // max - 1
            _ => max_positive,                   // max
        };
        return sign_extend(raw, bits);
    }

    // 3. Fallback: uniformly random value in the full signed range.
    let low = rng.u128(..);
    let high = rng.u128(..);
    let raw = U256::from(low) | (U256::from(high) << 128);
    sign_extend(raw, bits)
}

/// Compute the maximum positive value for a signed integer of `bits` width.
fn max_positive_for_bits(bits: usize) -> U256 {
    if bits == 0 {
        U256::ZERO
    } else if bits >= 256 {
        (U256::from(1) << 255) - U256::from(1)
    } else {
        (U256::from(1) << (bits - 1)) - U256::from(1)
    }
}

/// Compute the sign bit for a signed integer of `bits` width.
fn sign_bit(bits: usize) -> U256 {
    if bits == 0 {
        U256::ZERO
    } else if bits >= 256 {
        U256::from(1) << 255
    } else {
        U256::from(1) << (bits - 1)
    }
}

/// Compute the mask for the low `bits` bits.
fn mask(bits: usize) -> U256 {
    if bits == 0 {
        U256::ZERO
    } else if bits >= 256 {
        U256::MAX
    } else {
        (U256::from(1) << bits) - U256::from(1)
    }
}

/// Sign-extend a `bits`-wide raw unsigned value to a 256-bit signed integer.
fn sign_extend(raw: U256, bits: usize) -> I256 {
    if bits == 0 {
        return I256::ZERO;
    }

    let m = mask(bits);
    let value = raw & m;

    if bits >= 256 {
        // No sign extension needed; interpret the full 256 bits directly.
        return I256::from_raw(value);
    }

    let sb = sign_bit(bits);
    if value & sb == U256::ZERO {
        // Positive: high bits are already zero.
        I256::from_raw(value)
    } else {
        // Negative: set all bits above `bits` to 1.
        let extended = value | (U256::MAX ^ m);
        I256::from_raw(extended)
    }
}
