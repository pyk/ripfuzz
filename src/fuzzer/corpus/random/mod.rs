//! Random value generation helpers seeded with extracted literals.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, FixedBytes};
use fastrand::Rng;

pub use int::int;
pub use uint::uint;

use crate::fuzzer::corpus::ExtractedLiterals;

pub mod int;
pub mod uint;

/// Generate a random boolean value.
///
/// Returns `true` 50% of the time and `false` 50% of the time.
pub fn random_bool(rng: &mut Rng) -> bool {
    rng.bool()
}

/// Pick a random item from a slice, or return `None` if empty.
pub fn pick_random<T: Clone>(items: &[T], rng: &mut Rng) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(items[rng.usize(0..items.len())].clone())
    }
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
            DynSolType::Uint(sz) => DynSolValue::Uint(uint(*sz, literals, rng), *sz),
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
}
