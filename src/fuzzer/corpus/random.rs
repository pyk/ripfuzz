//! Random value generation helpers seeded with extracted literals.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, FixedBytes, I256, U256};
use fastrand::Rng;

use crate::fuzzer::corpus::ExtractedLiterals;

/// Generate a random boolean value.
///
/// Returns `true` 50% of the time and `false` 50% of the time.
pub fn random_bool(rng: &mut Rng) -> bool {
    rng.bool()
}

/// Compute the maximum value for an unsigned integer of `bits` width.
pub fn max_for_bits(bits: usize) -> U256 {
    if bits == 0 {
        U256::ZERO
    } else if bits >= 256 {
        U256::MAX
    } else {
        (U256::from(1) << bits) - U256::from(1)
    }
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
        return I256::from_raw(value);
    }

    let sb = sign_bit(bits);
    if value & sb == U256::ZERO {
        I256::from_raw(value)
    } else {
        let extended = value | (U256::MAX ^ m);
        I256::from_raw(extended)
    }
}

/// Pick a random item from a slice, or return `None` if empty.
pub fn pick_random<T: Clone>(rng: &mut Rng, items: &[T]) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(items[rng.usize(0..items.len())].clone())
    }
}

/// Generate a random unsigned integer of the given bit width.
///
/// Distribution:
/// - 20% chance to pick a literal from the extracted pool.
/// - 30% chance to generate an edge case (`0`, `1`, `max`, `max-1`,
///   `max-2`, `max-3`).
/// - 50% chance to generate a uniformly random value.
pub fn random_uint(rng: &mut Rng, bits: usize, literals: &ExtractedLiterals) -> U256 {
    let max = max_for_bits(bits);
    let group = literals.uint.get(&bits);

    let roll = rng.u32(0..100);
    if roll < 20 {
        if let Some(group) = group
            && !group.is_empty()
            && let Some(val) = pick_random(rng, group)
            && val <= max
        {
            return val;
        }
    } else if roll < 50 {
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

    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    let raw = U256::from_be_bytes::<32>(bytes);
    if bits == 256 { raw } else { raw & max }
}

/// Generate a random signed integer of the given bit width.
///
/// Distribution:
/// - 20% chance to pick a literal from the extracted pool.
/// - 30% chance to generate an edge case (`min`, `min+1`, `-1`, `0`,
///   `1`, `max-1`, `max`).
/// - 50% chance to generate a uniformly random value.
pub fn random_int(rng: &mut Rng, bits: usize, literals: &ExtractedLiterals) -> I256 {
    let max_positive = max_positive_for_bits(bits);
    let group = literals.int.get(&bits);

    let roll = rng.u32(0..100);
    if roll < 20 {
        if let Some(group) = group
            && !group.is_empty()
            && let Some(val) = pick_random(rng, group)
            && let Ok(u) = U256::try_from(val)
            && u <= max_positive
        {
            return val;
        }
    } else if roll < 50 {
        let raw = match rng.u32(0..7) {
            0 => sign_bit(bits),                 // min
            1 => sign_bit(bits) + U256::from(1), // min + 1
            2 => mask(bits),                     // -1
            3 => U256::ZERO,                     // 0
            4 => U256::from(1),                  // 1
            5 => max_positive - U256::from(1),   // max - 1
            _ => max_positive,                   // max
        };
        return sign_extend(raw, bits);
    }

    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    let raw = U256::from_be_bytes::<32>(bytes);
    sign_extend(raw, bits)
}

/// Generate a random fixed-width byte sequence.
///
/// `size` is the bit width (e.g. 32 for `bytes4`, 256 for `bytes32`).
///
/// Distribution:
/// - 20% chance to pick a literal from the extracted pool.
/// - 30% chance to generate an edge case.
/// - 50% chance to generate uniformly random bytes.
pub fn random_fixed_bytes(
    rng: &mut Rng,
    size: usize,
    literals: &ExtractedLiterals,
) -> FixedBytes<32> {
    let byte_len = size / 8;
    let group = literals.fixed_bytes.get(&size);

    let roll = rng.u32(0..100);
    if roll < 20 {
        if let Some(bucket) = group
            && !bucket.is_empty()
            && let Some(val) = pick_random(rng, bucket)
        {
            return val;
        }
    } else if roll < 50 {
        let mut word = [0u8; 32];
        match rng.u32(0..6) {
            0 => {}                           // all zeros
            1 => word[0] = 1,                 // 1 in first byte
            2 => word[..byte_len].fill(0xFF), // max
            3 => {
                word[..byte_len].fill(0xFF);
                word[byte_len.saturating_sub(1)] = 0xFE; // max - 1
            }
            4 => {
                word[..byte_len].fill(0xFF);
                word[byte_len.saturating_sub(1)] = 0xFD; // max - 2
            }
            _ => {
                word[..byte_len].fill(0xFF);
                word[byte_len.saturating_sub(1)] = 0xFC; // max - 3
            }
        }
        return FixedBytes::from(word);
    }

    let mut word = [0u8; 32];
    rng.fill(&mut word);
    FixedBytes::from(word)
}

/// Generate a random Ethereum address.
///
/// Distribution:
/// - 20% chance to pick a literal from the extracted pool.
/// - 30% chance to generate an edge case.
/// - 50% chance to generate uniformly random bytes.
pub fn random_address(rng: &mut Rng, literals: &ExtractedLiterals) -> Address {
    let roll = rng.u32(0..100);
    if roll < 20 {
        if let Some(val) = pick_random(rng, &literals.address) {
            return val;
        }
    } else if roll < 50 {
        let mut bytes = [0u8; 20];
        match rng.u32(0..6) {
            0 => {}                // all zeros
            1 => bytes[0] = 1,     // 1 in first byte
            2 => bytes.fill(0xFF), // max
            3 => {
                bytes.fill(0xFF);
                bytes[19] = 0xFE; // max - 1
            }
            4 => {
                bytes.fill(0xFF);
                bytes[19] = 0xFD; // max - 2
            }
            _ => {
                bytes.fill(0xFF);
                bytes[19] = 0xFC; // max - 3
            }
        }
        return Address::from_slice(&bytes);
    }

    let mut bytes = [0u8; 20];
    rng.fill(&mut bytes);
    Address::from_slice(&bytes)
}

/// Extension trait to generate random [`DynSolValue`]s for a given type.
pub trait RandomDynSolValue {
    /// Generate a random value of this Solidity type.
    fn random(&self, rng: &mut Rng, literals: &ExtractedLiterals) -> DynSolValue;
}

impl RandomDynSolValue for DynSolType {
    fn random(&self, rng: &mut Rng, literals: &ExtractedLiterals) -> DynSolValue {
        match self {
            DynSolType::Bool => DynSolValue::Bool(random_bool(rng)),
            DynSolType::Uint(sz) => DynSolValue::Uint(random_uint(rng, *sz, literals), *sz),
            DynSolType::Int(sz) => DynSolValue::Int(random_int(rng, *sz, literals), *sz),
            DynSolType::FixedBytes(sz) => {
                DynSolValue::FixedBytes(random_fixed_bytes(rng, *sz, literals), *sz)
            }
            DynSolType::Address => DynSolValue::Address(random_address(rng, literals)),
            DynSolType::Bytes => {
                if let Some(val) = pick_random(rng, &literals.bytes) {
                    return DynSolValue::Bytes(val.to_vec());
                }
                let len = rng.usize(0..=64);
                let mut bytes = vec![0u8; len];
                rng.fill(&mut bytes);
                DynSolValue::Bytes(bytes)
            }
            DynSolType::String => {
                if let Some(val) = pick_random(rng, &literals.string) {
                    return DynSolValue::String(val);
                }
                let len = rng.usize(0..=32);
                let s: String = (0..len).map(|_| rng.alphabetic()).collect();
                DynSolValue::String(s)
            }
            DynSolType::Function => {
                let mut bytes = [0u8; 24];
                rng.fill(&mut bytes);
                DynSolValue::Function(alloy_primitives::Function::from_slice(&bytes))
            }
            DynSolType::Array(inner) => {
                let len = rng.usize(0..=4);
                let arr: Vec<DynSolValue> = (0..len).map(|_| inner.random(rng, literals)).collect();
                DynSolValue::Array(arr)
            }
            DynSolType::FixedArray(inner, len) => {
                let arr: Vec<DynSolValue> =
                    (0..*len).map(|_| inner.random(rng, literals)).collect();
                DynSolValue::FixedArray(arr)
            }
            DynSolType::Tuple(types) => {
                let values: Vec<DynSolValue> =
                    types.iter().map(|t| t.random(rng, literals)).collect();
                DynSolValue::Tuple(values)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;

    #[test]
    fn max_for_bits_zero() {
        assert_eq!(max_for_bits(0), U256::ZERO);
    }

    #[test]
    fn max_for_bits_8() {
        assert_eq!(max_for_bits(8), U256::from(0xFF));
    }

    #[test]
    fn max_for_bits_256() {
        assert_eq!(max_for_bits(256), U256::MAX);
    }

    #[test]
    fn max_for_bits_over_256() {
        assert_eq!(max_for_bits(512), U256::MAX);
    }

    #[test]
    fn random_bool_distribution_is_fifty_fifty() {
        let mut rng = fastrand::Rng::with_seed(42);
        let mut true_count = 0usize;
        let total = 1_000usize;

        for _ in 0..total {
            if random_bool(&mut rng) {
                true_count += 1;
            }
        }

        assert!(
            true_count > 400 && true_count < 600,
            "expected ~50% true, got {true_count} / {total}"
        );
    }

    #[test]
    fn random_uint_distribution_matches_spec() {
        let mut rng = fastrand::Rng::with_seed(123);
        let bits = 256;
        let max = max_for_bits(bits);

        let mut literals = ExtractedLiterals::default();
        literals.uint.insert(
            bits,
            vec![U256::from(42), U256::from(100), U256::from(12345)],
        );

        let total = 1_000usize;
        let mut literal_count = 0usize;
        let mut edge_count = 0usize;

        let edge_cases = [
            U256::ZERO,
            U256::from(1),
            max,
            max.saturating_sub(U256::from(1)),
            max.saturating_sub(U256::from(2)),
            max.saturating_sub(U256::from(3)),
        ];

        for _ in 0..total {
            let val = random_uint(&mut rng, bits, &literals);
            let is_literal = literals.uint.get(&bits).unwrap().contains(&val);
            let is_edge = edge_cases.contains(&val);

            if is_literal {
                literal_count += 1;
            } else if is_edge {
                edge_count += 1;
            }
        }

        let random_count = total - literal_count - edge_count;

        assert!(
            literal_count > 100 && literal_count < 300,
            "expected ~20% literals, got {literal_count} / {total}"
        );
        assert!(
            edge_count > 200 && edge_count < 400,
            "expected ~30% edge cases, got {edge_count} / {total}"
        );
        assert!(
            random_count > 400 && random_count < 600,
            "expected ~50% random, got {random_count} / {total}"
        );
    }

    #[test]
    fn random_int_distribution_matches_spec() {
        let mut rng = fastrand::Rng::with_seed(456);
        let bits = 256;
        let max_pos = max_positive_for_bits(bits);

        let mut literals = ExtractedLiterals::default();
        literals.int.insert(
            bits,
            vec![
                I256::from_raw(U256::from(42)),
                I256::from_raw(U256::from(100)),
                I256::from_raw(U256::from(12345)),
            ],
        );

        let total = 1_000usize;
        let mut literal_count = 0usize;
        let mut edge_count = 0usize;

        let edge_cases = [
            sign_extend(sign_bit(bits), bits),                 // min
            sign_extend(sign_bit(bits) + U256::from(1), bits), // min + 1
            sign_extend(mask(bits), bits),                     // -1
            I256::ZERO,                                        // 0
            I256::from_raw(U256::from(1)),                     // 1
            sign_extend(max_pos - U256::from(1), bits),        // max - 1
            sign_extend(max_pos, bits),                        // max
        ];

        for _ in 0..total {
            let val = random_int(&mut rng, bits, &literals);
            let is_literal = literals.int.get(&bits).unwrap().contains(&val);
            let is_edge = edge_cases.contains(&val);

            if is_literal {
                literal_count += 1;
            } else if is_edge {
                edge_count += 1;
            }
        }

        let random_count = total - literal_count - edge_count;

        assert!(
            literal_count > 100 && literal_count < 300,
            "expected ~20% literals, got {literal_count} / {total}"
        );
        assert!(
            edge_count > 200 && edge_count < 400,
            "expected ~30% edge cases, got {edge_count} / {total}"
        );
        assert!(
            random_count > 400 && random_count < 600,
            "expected ~50% random, got {random_count} / {total}"
        );
    }

    #[test]
    fn random_fixed_bytes_distribution_matches_spec() {
        let mut rng = fastrand::Rng::with_seed(789);
        let size = 256;
        let byte_len = size / 8;

        let mut literals = ExtractedLiterals::default();
        let mut lit_word = [0u8; 32];
        lit_word[0] = 0xAB;
        lit_word[1] = 0xCD;
        literals
            .fixed_bytes
            .insert(size, vec![FixedBytes::from(lit_word)]);

        let total = 1_000usize;
        let mut literal_count = 0usize;
        let mut edge_count = 0usize;

        let edge_cases = [
            FixedBytes::from([0u8; 32]),
            {
                let mut w = [0u8; 32];
                w[0] = 1;
                FixedBytes::from(w)
            },
            {
                let mut w = [0u8; 32];
                w[..byte_len].fill(0xFF);
                FixedBytes::from(w)
            },
            {
                let mut w = [0u8; 32];
                w[..byte_len].fill(0xFF);
                w[byte_len - 1] = 0xFE;
                FixedBytes::from(w)
            },
            {
                let mut w = [0u8; 32];
                w[..byte_len].fill(0xFF);
                w[byte_len - 1] = 0xFD;
                FixedBytes::from(w)
            },
            {
                let mut w = [0u8; 32];
                w[..byte_len].fill(0xFF);
                w[byte_len - 1] = 0xFC;
                FixedBytes::from(w)
            },
        ];

        for _ in 0..total {
            let val = random_fixed_bytes(&mut rng, size, &literals);
            let is_literal = literals.fixed_bytes.get(&size).unwrap().contains(&val);
            let is_edge = edge_cases.contains(&val);

            if is_literal {
                literal_count += 1;
            } else if is_edge {
                edge_count += 1;
            }
        }

        let random_count = total - literal_count - edge_count;

        assert!(
            literal_count > 100 && literal_count < 300,
            "expected ~20% literals, got {literal_count} / {total}"
        );
        assert!(
            edge_count > 200 && edge_count < 400,
            "expected ~30% edge cases, got {edge_count} / {total}"
        );
        assert!(
            random_count > 400 && random_count < 600,
            "expected ~50% random, got {random_count} / {total}"
        );
    }

    #[test]
    fn random_address_distribution_matches_spec() {
        let mut rng = fastrand::Rng::with_seed(101);

        let mut literals = ExtractedLiterals::default();
        let lit_addr = Address::from([0xAB; 20]);
        literals.address.push(lit_addr);

        let total = 1_000usize;
        let mut literal_count = 0usize;
        let mut edge_count = 0usize;

        let edge_cases = [
            address!("0x0000000000000000000000000000000000000000"),
            address!("0x0100000000000000000000000000000000000000"),
            address!("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF"),
            address!("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFE"),
            address!("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFD"),
            address!("0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFC"),
        ];

        for _ in 0..total {
            let val = random_address(&mut rng, &literals);
            let is_literal = literals.address.contains(&val);
            let is_edge = edge_cases.contains(&val);

            if is_literal {
                literal_count += 1;
            } else if is_edge {
                edge_count += 1;
            }
        }

        let random_count = total - literal_count - edge_count;

        assert!(
            literal_count > 100 && literal_count < 300,
            "expected ~20% literals, got {literal_count} / {total}"
        );
        assert!(
            edge_count > 200 && edge_count < 400,
            "expected ~30% edge cases, got {edge_count} / {total}"
        );
        assert!(
            random_count > 400 && random_count < 600,
            "expected ~50% random, got {random_count} / {total}"
        );
    }
}
