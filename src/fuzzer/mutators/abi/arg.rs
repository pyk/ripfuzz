//! ABI argument mutator that perturbs decoded Solidity values.

use alloy_dyn_abi::DynSolValue;
use alloy_primitives::{Address, I256, U256};

use crate::fuzzer::corpus::Call;
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Mutate the arguments of a random call using ABI type information.
///
/// This mutator works directly on the [`DynSolValue`]s stored in each
/// [`Call`], mutates them recursively with type-aware rules, and leaves
/// the re-encoding to the caller (or to [`Call::encode`]).
#[derive(Debug, Default)]
pub struct SequenceArgMutator;

impl SequenceArgMutator {
    pub fn new() -> Self {
        Self
    }

    /// Mutate the arguments of a single call.
    ///
    /// Returns `true` if any argument was changed.
    fn mutate_call_args(&self, rng: &mut fastrand::Rng, call: &mut Call) -> bool {
        let values = match &mut call.args {
            DynSolValue::Tuple(elems) => elems,
            _ => return false,
        };

        let mut mutated = false;
        for elem in values.iter_mut() {
            mutated |= self.mutate_value(rng, elem);
        }

        mutated
    }

    /// Recursively mutate a single [`DynSolValue`].
    ///
    /// Returns `true` if the value was changed.
    fn mutate_value(&self, rng: &mut fastrand::Rng, value: &mut DynSolValue) -> bool {
        match value {
            DynSolValue::Uint(v, sz) => {
                let delta = (rng.u64(0..1_000)) as i64 - 500;
                let delta_u256 = U256::from(delta.unsigned_abs() as u128);
                *v = if delta >= 0 {
                    v.wrapping_add(delta_u256)
                } else {
                    v.wrapping_sub(delta_u256)
                };
                if *sz < 256 {
                    let mask: U256 = (U256::from(1u8) << *sz).wrapping_sub(U256::from(1u8));
                    *v &= mask;
                }
                true
            }
            DynSolValue::Int(v, _sz) => {
                let delta = (rng.u64(0..1_000)) as i64 - 500;
                let delta_i256 = match I256::try_from(delta) {
                    Ok(d) => d,
                    Err(_) => return false,
                };
                *v = if delta >= 0 {
                    v.wrapping_add(delta_i256)
                } else {
                    v.wrapping_sub(delta_i256)
                };
                true
            }
            DynSolValue::Bool(b) => {
                *b = !*b;
                true
            }
            DynSolValue::Address(a) => {
                let mut bytes = a.to_vec();
                for byte in bytes.iter_mut().skip(12) {
                    *byte = rng.u8(..);
                }
                *a = Address::from_slice(&bytes);
                true
            }
            DynSolValue::Function(f) => {
                let mut bytes = f.as_slice().to_vec();
                if bytes.is_empty() {
                    return false;
                }
                let idx = rng.usize(0..bytes.len());
                bytes[idx] = rng.u8(..);
                *f = alloy_primitives::Function::from_slice(&bytes);
                true
            }
            DynSolValue::Bytes(b) => {
                if b.is_empty() {
                    return false;
                }
                let idx = rng.usize(0..b.len());
                b[idx] = rng.u8(..);
                true
            }
            DynSolValue::String(s) => {
                let mut bytes = s.as_bytes().to_vec();
                if bytes.is_empty() {
                    return false;
                }
                let idx = rng.usize(0..bytes.len());
                bytes[idx] = rng.u8(..);
                // Lossy conversion is safe: Solidity strings are byte
                // sequences and do not require valid UTF-8.
                *s = String::from_utf8_lossy(&bytes).into_owned();
                true
            }
            DynSolValue::FixedBytes(word, sz) => {
                let bytes = word.as_mut_slice();
                if *sz == 0 {
                    return false;
                }
                let idx = rng.usize(0..*sz);
                bytes[idx] = rng.u8(..);
                true
            }
            DynSolValue::Array(arr) | DynSolValue::FixedArray(arr) => {
                let mut sub = false;
                if arr.len() >= 2 && rng.u64(0..4) == 0 {
                    let i = rng.usize(0..arr.len());
                    let j = rng.usize(0..arr.len());
                    arr.swap(i, j);
                    sub = true;
                }
                for elem in arr.iter_mut() {
                    sub |= self.mutate_value(rng, elem);
                }
                sub
            }
            DynSolValue::Tuple(arr) => {
                let mut sub = false;
                for elem in arr.iter_mut() {
                    sub |= self.mutate_value(rng, elem);
                }
                sub
            }
        }
    }
}

impl Mutator for SequenceArgMutator {
    fn mutate(&self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
        if calls.is_empty() {
            return MutationResult::Skipped;
        }
        let call_idx = rng.usize(0..calls.len());

        if self.mutate_call_args(rng, &mut calls[call_idx]) {
            MutationResult::Mutated
        } else {
            MutationResult::Skipped
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_json_abi::Function;
    use alloy_primitives::{Address, I256, U256};

    use crate::fuzzer::corpus;
    use crate::fuzzer::mutators::Mutator;
    use crate::fuzzer::mutators::abi;

    #[test]
    fn empty_sequence_is_skipped() {
        let mut rng = fastrand::Rng::with_seed(42);
        let mutator = abi::SequenceArgMutator::new();
        let mut calls = Vec::new();

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Skipped);
    }

    #[test]
    fn uint256_argument_is_mutated() {
        let func = Function::parse("set(uint256)").unwrap();

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mutator = abi::SequenceArgMutator::new();
            let mut calls = vec![corpus::Call {
                function: func.clone(),
                args: DynSolValue::Tuple(vec![DynSolValue::Uint(U256::ZERO, 256)]),
                ..Default::default()
            }];
            let original = calls[0].calldata()[4..].to_vec();

            let result = mutator.mutate(&mut rng, &mut calls);
            assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
            if calls[0].calldata()[4..] != original {
                any_changed = true;
            }
        }
        assert!(
            any_changed,
            "at least one seed should change the uint256 bytes"
        );
    }

    #[test]
    fn uint256_mutation_affects_all_256_bits() {
        let func = Function::parse("set(uint256)").unwrap();

        let mut any_high_changed = false;
        let mut any_low_changed = false;
        for seed in [1u64, 2, 3, 42, 99, 123, 456, 789, 1000, 2000] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mutator = abi::SequenceArgMutator::new();
            let mut calls = vec![corpus::Call {
                function: func.clone(),
                args: DynSolValue::Tuple(vec![DynSolValue::Uint(
                    U256::from_be_bytes([
                        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
                        0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    ]),
                    256,
                )]),
                ..Default::default()
            }];
            let original = calls[0].calldata()[4..].to_vec();

            mutator.mutate(&mut rng, &mut calls);

            let mutated = calls[0].calldata()[4..].to_vec();
            if mutated[..16] != original[..16] {
                any_high_changed = true;
            }
            if mutated[16..] != original[16..] {
                any_low_changed = true;
            }
        }
        assert!(any_high_changed, "high 128 bits should be mutable");
        assert!(any_low_changed, "low 128 bits should be mutable");
    }

    #[test]
    fn dynamic_bytes_type_is_properly_handled() {
        let func = Function::parse("setData(bytes)").unwrap();

        let mut rng = fastrand::Rng::with_seed(42);
        let mutator = abi::SequenceArgMutator::new();
        let mut calls = vec![corpus::Call {
            function: func.clone(),
            args: DynSolValue::Tuple(vec![DynSolValue::Bytes(vec![0xab, 0xcd])]),
            ..Default::default()
        }];
        let original = calls[0].calldata()[4..].to_vec();

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);

        let mutated = calls[0].calldata()[4..].to_vec();

        assert_eq!(
            &mutated[..64],
            &original[..64],
            "offset and length must not be corrupted"
        );
        assert_ne!(
            &mutated[64..],
            &original[64..],
            "dynamic data should have been mutated"
        );
    }

    #[test]
    fn bool_argument_is_flipped() {
        let mut rng = fastrand::Rng::with_seed(42);
        let mutator = abi::SequenceArgMutator::new();
        let mut calls = vec![corpus::Call {
            function: Function::parse("toggle(bool)").unwrap(),
            args: DynSolValue::Tuple(vec![DynSolValue::Bool(true)]),
            ..Default::default()
        }];

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
        assert_eq!(
            calls[0].args.as_tuple().unwrap()[0].as_bool().unwrap(),
            false,
            "flipped to false"
        );
    }

    #[test]
    fn address_argument_is_mutated() {
        let mut rng = fastrand::Rng::with_seed(42);
        let mutator = abi::SequenceArgMutator::new();
        let mut calls = vec![corpus::Call {
            function: Function::parse("transfer(address)").unwrap(),
            args: DynSolValue::Tuple(vec![DynSolValue::Address(Address::ZERO)]),
            ..Default::default()
        }];
        let original = calls[0].calldata()[4..].to_vec();

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
        let mutated = calls[0].calldata()[4..].to_vec();
        assert_ne!(&mutated[12..32], &original[12..32]);
    }

    #[test]
    fn address_mutation_overwrites_all_20_bytes() {
        let func = Function::parse("transfer(address)").unwrap();

        for seed in [1u64, 2, 3, 42, 99, 123, 456, 789, 1000, 2000] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mutator = abi::SequenceArgMutator::new();
            let mut calls = vec![corpus::Call {
                function: func.clone(),
                args: DynSolValue::Tuple(vec![DynSolValue::Address(Address::ZERO)]),
                ..Default::default()
            }];

            mutator.mutate(&mut rng, &mut calls);
            let mutated = &calls[0].calldata()[4..];

            assert_eq!(&mutated[..12], &[0u8; 12], "address padding must stay zero");
            assert_ne!(
                &mutated[12..32],
                &[0u8; 20],
                "all 20 address bytes should be randomized"
            );
        }
    }

    #[test]
    fn multiple_arguments_all_get_mutated() {
        let mut rng = fastrand::Rng::with_seed(123);
        let mutator = abi::SequenceArgMutator::new();
        let mut calls = vec![corpus::Call {
            function: Function::parse("multi(uint256,bool,address)").unwrap(),
            args: DynSolValue::Tuple(vec![
                DynSolValue::Uint(U256::ZERO, 256),
                DynSolValue::Bool(false),
                DynSolValue::Address(Address::ZERO),
            ]),
            ..Default::default()
        }];
        let original = calls[0].calldata()[4..].to_vec();

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
        assert_ne!(calls[0].calldata()[4..], original);
    }

    #[test]
    fn repeated_mutation_produces_different_values() {
        let func = Function::parse("set(uint256)").unwrap();

        let mut values = Vec::new();
        for seed in [1u64, 2, 3, 4, 5] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mutator = abi::SequenceArgMutator::new();
            let mut calls = vec![corpus::Call {
                function: func.clone(),
                args: DynSolValue::Tuple(vec![DynSolValue::Uint(U256::ZERO, 256)]),
                ..Default::default()
            }];
            mutator.mutate(&mut rng, &mut calls);
            values.push(calls[0].calldata()[4..].to_vec());
        }

        let first = &values[0];
        let all_same = values.iter().all(|v| v == first);
        assert!(!all_same, "mutations with different seeds should vary");
    }

    #[test]
    fn uint8_argument_is_masked() {
        let mut rng = fastrand::Rng::with_seed(1);
        let mutator = abi::SequenceArgMutator::new();
        let mut calls = vec![corpus::Call {
            function: Function::parse("set(uint8)").unwrap(),
            args: DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(255), 8)]),
            ..Default::default()
        }];

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
        let decoded = DynSolType::Tuple(vec![DynSolType::Uint(8)])
            .abi_decode_params(&calls[0].calldata()[4..])
            .unwrap();
        if let DynSolValue::Tuple(v) = decoded {
            if let DynSolValue::Uint(n, 8) = v[0] {
                assert!(n <= U256::from(255), "uint8 must stay in [0, 255]");
            }
        }
    }

    #[test]
    fn int256_argument_is_mutated() {
        let func = Function::parse("set(int256)").unwrap();

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mutator = abi::SequenceArgMutator::new();
            let mut calls = vec![corpus::Call {
                function: func.clone(),
                args: DynSolValue::Tuple(vec![DynSolValue::Int(I256::ZERO, 256)]),
                ..Default::default()
            }];
            let original = calls[0].calldata()[4..].to_vec();

            let result = mutator.mutate(&mut rng, &mut calls);
            assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
            if calls[0].calldata()[4..] != original {
                any_changed = true;
            }
        }
        assert!(
            any_changed,
            "at least one seed should change the int256 bytes"
        );
    }

    #[test]
    fn tuple_argument_is_mutated() {
        let func = Function::parse("set((uint256,bool))").unwrap();

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mutator = abi::SequenceArgMutator::new();
            let mut calls = vec![corpus::Call {
                function: func.clone(),
                args: DynSolValue::Tuple(vec![DynSolValue::Tuple(vec![
                    DynSolValue::Uint(U256::ZERO, 256),
                    DynSolValue::Bool(false),
                ])]),
                ..Default::default()
            }];
            let original = calls[0].calldata()[4..].to_vec();
            mutator.mutate(&mut rng, &mut calls);
            if calls[0].calldata()[4..] != original {
                any_changed = true;
            }
        }
        assert!(any_changed, "tuple fields should be mutable");
    }

    #[test]
    fn string_argument_is_mutated() {
        let mut rng = fastrand::Rng::with_seed(42);
        let mutator = abi::SequenceArgMutator::new();
        let mut calls = vec![corpus::Call {
            function: Function::parse("set(string)").unwrap(),
            args: DynSolValue::Tuple(vec![DynSolValue::String("hello".into())]),
            ..Default::default()
        }];

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);

        let decoded = DynSolType::Tuple(vec![DynSolType::String])
            .abi_decode_params(&calls[0].calldata()[4..])
            .unwrap();
        if let DynSolValue::Tuple(values) = decoded {
            if let DynSolValue::String(s) = &values[0] {
                assert_ne!(s, "hello", "string should be mutated");
            } else {
                panic!("expected String");
            }
        } else {
            panic!("expected Tuple");
        }
    }

    #[test]
    fn function_pointer_argument_is_mutated() {
        let func = Function::parse("set(function)").unwrap();
        let func_val = alloy_primitives::Function::from_slice(&[0u8; 24]);

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mutator = abi::SequenceArgMutator::new();
            let mut calls = vec![corpus::Call {
                function: func.clone(),
                args: DynSolValue::Tuple(vec![DynSolValue::Function(func_val)]),
                ..Default::default()
            }];
            let original = calls[0].calldata()[4..].to_vec();
            mutator.mutate(&mut rng, &mut calls);
            if calls[0].calldata()[4..] != original {
                any_changed = true;
            }
        }
        assert!(any_changed, "function pointer should be mutable");
    }
}
