//! Random value generation helpers seeded with extracted literals.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, FixedBytes, U256};
use fastrand::Rng;

pub use int::int;

use crate::fuzzer::corpus::ExtractedLiterals;

pub mod int;

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

/// Pick a random item from a slice, or return `None` if empty.
pub fn pick_random<T: Clone>(items: &[T], rng: &mut Rng) -> Option<T> {
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
            && let Some(val) = pick_random(group, rng)
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
            DynSolType::Int(sz) => DynSolValue::Int(int(*sz, literals, rng), *sz),
            DynSolType::FixedBytes(sz) => {
                if let Some(bucket) = literals.fixed_bytes.get(sz)
                    && let Some(val) = pick_random(bucket, rng)
                {
                    return DynSolValue::FixedBytes(val, *sz);
                }
                let mut word = [0u8; 32];
                rng.fill(&mut word);
                DynSolValue::FixedBytes(FixedBytes::from(word), *sz)
            }
            DynSolType::Address => {
                if let Some(val) = pick_random(&literals.address, rng) {
                    return DynSolValue::Address(val);
                }
                let mut bytes = [0u8; 20];
                rng.fill(&mut bytes);
                DynSolValue::Address(Address::from_slice(&bytes))
            }
            DynSolType::Bytes => {
                if let Some(val) = pick_random(&literals.bytes, rng) {
                    return DynSolValue::Bytes(val.to_vec());
                }
                let len = rng.usize(0..=64);
                let mut bytes = vec![0u8; len];
                rng.fill(&mut bytes);
                DynSolValue::Bytes(bytes)
            }
            DynSolType::String => {
                if let Some(val) = pick_random(&literals.string, rng) {
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
}
