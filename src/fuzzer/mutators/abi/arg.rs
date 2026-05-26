//! ABI argument mutator that perturbs decoded Solidity values.

use std::collections::HashMap;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, I256, U256};

use crate::fuzzer::corpus::Call;
use crate::fuzzer::mutators::{MutationResult, Mutator};

/// Mutate the arguments of a random call using ABI type information.
///
/// This mutator decodes the raw ABI buffer into [`DynSolValue`]s, mutates
/// them recursively with type-aware rules, and re-encodes the result.
/// Composite types (arrays, tuples, structs) are supported.
#[derive(Debug)]
pub struct SequenceArgMutator {
    /// Pre-built map from selector to parsed tuple type (ready to decode).
    selector_types: HashMap<[u8; 4], DynSolType>,
}

impl SequenceArgMutator {
    pub fn new(abi: JsonAbi) -> Self {
        let mut selector_types = HashMap::new();
        for func in abi.functions() {
            let sel: [u8; 4] = func.selector().into();
            let types: Vec<DynSolType> = func
                .inputs
                .iter()
                .filter_map(|p| p.selector_type().parse::<DynSolType>().ok())
                .collect();
            selector_types.insert(sel, DynSolType::Tuple(types));
        }
        Self { selector_types }
    }

    /// Mutate the arguments of a single call.
    ///
    /// Returns `true` if any argument was changed.
    fn mutate_call_args(&self, rng: &mut fastrand::Rng, call: &mut Call) -> bool {
        let tuple_type = match self.selector_types.get(&call.selector) {
            Some(t) => t.clone(),
            None => return false,
        };

        let mut values = match tuple_type.abi_decode_params(&call.args) {
            Ok(v) => v,
            Err(_) => return false,
        };

        let mut mutated = false;
        if let DynSolValue::Tuple(ref mut elems) = values {
            for elem in elems.iter_mut() {
                mutated |= self.mutate_value(rng, elem);
            }
        }

        if mutated {
            call.args = values.abi_encode_params();
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
    fn mutate(&mut self, rng: &mut fastrand::Rng, calls: &mut Vec<Call>) -> MutationResult {
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
    use alloy_primitives::U256;

    use crate::fuzzer::corpus;
    use crate::fuzzer::mutators::Mutator;
    use crate::fuzzer::mutators::abi;

    fn abi_with(function_sig: &str) -> alloy_json_abi::JsonAbi {
        alloy_json_abi::JsonAbi::parse([function_sig]).unwrap()
    }

    fn selector_of(abi: &alloy_json_abi::JsonAbi, name: &str) -> [u8; 4] {
        abi.functions()
            .find(|f| f.name == name)
            .unwrap()
            .selector()
            .into()
    }

    #[test]
    fn empty_sequence_is_skipped() {
        let mut rng = fastrand::Rng::with_seed(42);
        let abi = abi_with("function set(uint256 x)");
        let mut mutator = abi::SequenceArgMutator::new(abi);
        let mut calls = Vec::new();

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Skipped);
    }

    #[test]
    fn unknown_selector_returns_skipped() {
        let mut rng = fastrand::Rng::with_seed(42);
        let target_abi = abi_with("function set(uint256 x)");
        let other_abi = abi_with("function transfer(address to)");
        let unknown_selector = selector_of(&other_abi, "transfer");
        let mut mutator = abi::SequenceArgMutator::new(target_abi);

        let mut calls = vec![corpus::Call {
            selector: unknown_selector,
            args: vec![0u8; 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];
        let original_args = calls[0].args.clone();

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Skipped);
        assert_eq!(calls[0].args, original_args, "args must be unchanged");
    }

    #[test]
    fn uint256_argument_is_mutated() {
        let abi = abi_with("function set(uint256 x)");
        let selector = selector_of(&abi, "set");

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut calls = vec![corpus::Call {
                selector,
                args: vec![0u8; 32],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }];
            let original_args = calls[0].args.clone();

            let result = mutator.mutate(&mut rng, &mut calls);
            assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
            if calls[0].args != original_args {
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
        let abi = abi_with("function set(uint256 x)");
        let selector = selector_of(&abi, "set");

        let high_bytes = vec![0xFFu8; 16];
        let low_bytes = vec![0u8; 16];
        let mut full_arg = high_bytes.clone();
        full_arg.extend_from_slice(&low_bytes);

        let mut any_high_changed = false;
        let mut any_low_changed = false;
        for seed in [1u64, 2, 3, 42, 99, 123, 456, 789, 1000, 2000] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut calls = vec![corpus::Call {
                selector,
                args: full_arg.clone(),
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }];

            mutator.mutate(&mut rng, &mut calls);

            if calls[0].args[..16] != high_bytes[..] {
                any_high_changed = true;
            }
            if calls[0].args[16..] != low_bytes[..] {
                any_low_changed = true;
            }
        }
        assert!(any_high_changed, "high 128 bits should be mutable");
        assert!(any_low_changed, "low 128 bits should be mutable");
    }

    #[test]
    fn dynamic_bytes_type_is_properly_handled() {
        let abi = abi_with("function setData(bytes data)");
        let selector = selector_of(&abi, "setData");

        let mut args = vec![0u8; 96];
        args[31] = 0x20; // offset = 32
        args[63] = 0x02; // length = 2
        args[64] = 0xab; // data byte 0
        args[65] = 0xcd; // data byte 1

        let mut rng = fastrand::Rng::with_seed(42);
        let mut mutator = abi::SequenceArgMutator::new(abi);
        let mut calls = vec![corpus::Call {
            selector,
            args: args.clone(),
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);

        let mutated = &calls[0].args;

        assert_eq!(
            &mutated[..64],
            &args[..64],
            "offset and length must not be corrupted"
        );
        assert_ne!(
            &mutated[64..],
            &args[64..],
            "dynamic data should have been mutated"
        );
    }

    #[test]
    fn bool_argument_is_flipped() {
        let mut rng = fastrand::Rng::with_seed(42);
        let abi = abi_with("function toggle(bool b)");
        let selector = selector_of(&abi, "toggle");
        let mut mutator = abi::SequenceArgMutator::new(abi);

        let mut calls = vec![corpus::Call {
            selector,
            args: {
                let mut v = vec![0u8; 32];
                v[31] = 1; // true
                v
            },
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
        assert_eq!(calls[0].args[31], 0); // flipped to false
    }

    #[test]
    fn address_argument_is_mutated() {
        let mut rng = fastrand::Rng::with_seed(42);
        let abi = abi_with("function transfer(address to)");
        let selector = selector_of(&abi, "transfer");
        let mut mutator = abi::SequenceArgMutator::new(abi);

        let mut calls = vec![corpus::Call {
            selector,
            args: vec![0u8; 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];
        let original_args = calls[0].args.clone();

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
        assert_ne!(&calls[0].args[12..32], &original_args[12..32]);
    }

    #[test]
    fn address_mutation_overwrites_all_20_bytes() {
        let abi = abi_with("function transfer(address to)");
        let selector = selector_of(&abi, "transfer");

        for seed in [1u64, 2, 3, 42, 99, 123, 456, 789, 1000, 2000] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut calls = vec![corpus::Call {
                selector,
                args: vec![0u8; 32],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }];

            mutator.mutate(&mut rng, &mut calls);
            let mutated = &calls[0].args;

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
        let abi = abi_with("function multi(uint256 a, bool b, address c)");
        let selector = selector_of(&abi, "multi");
        let mut mutator = abi::SequenceArgMutator::new(abi);

        let mut calls = vec![corpus::Call {
            selector,
            args: vec![0u8; 96],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];
        let original_args = calls[0].args.clone();

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
        assert_ne!(calls[0].args, original_args);
    }

    #[test]
    fn repeated_mutation_produces_different_values() {
        let abi = abi_with("function set(uint256 x)");
        let selector = selector_of(&abi, "set");

        let mut values = Vec::new();
        for seed in [1u64, 2, 3, 4, 5] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut calls = vec![corpus::Call {
                selector,
                args: vec![0u8; 32],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }];
            mutator.mutate(&mut rng, &mut calls);
            values.push(calls[0].args.clone());
        }

        let first = &values[0];
        let all_same = values.iter().all(|v| v == first);
        assert!(!all_same, "mutations with different seeds should vary");
    }

    #[test]
    fn uint8_argument_is_masked() {
        let abi = abi_with("function set(uint8 x)");
        let selector = selector_of(&abi, "set");

        let args =
            DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(255), 8)]).abi_encode_params();

        let mut rng = fastrand::Rng::with_seed(1);
        let mut mutator = abi::SequenceArgMutator::new(abi);
        let mut calls = vec![corpus::Call {
            selector,
            args: args.clone(),
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
        let decoded = DynSolType::Tuple(vec![DynSolType::Uint(8)])
            .abi_decode_params(&calls[0].args)
            .unwrap();
        if let DynSolValue::Tuple(v) = decoded {
            if let DynSolValue::Uint(n, 8) = v[0] {
                assert!(n <= U256::from(255), "uint8 must stay in [0, 255]");
            }
        }
    }

    #[test]
    fn int256_argument_is_mutated() {
        let abi = abi_with("function set(int256 x)");
        let selector = selector_of(&abi, "set");

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut calls = vec![corpus::Call {
                selector,
                args: vec![0u8; 32],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }];
            let original_args = calls[0].args.clone();

            let result = mutator.mutate(&mut rng, &mut calls);
            assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);
            if calls[0].args != original_args {
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
        let abi = abi_with("function set((uint256,bool) x)");
        let selector = selector_of(&abi, "set");

        let args = DynSolValue::Tuple(vec![DynSolValue::Tuple(vec![
            DynSolValue::Uint(U256::ZERO, 256),
            DynSolValue::Bool(false),
        ])])
        .abi_encode_params();

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut calls = vec![corpus::Call {
                selector,
                args: args.clone(),
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }];
            mutator.mutate(&mut rng, &mut calls);
            if calls[0].args != args {
                any_changed = true;
            }
        }
        assert!(any_changed, "tuple fields should be mutable");
    }

    #[test]
    fn string_argument_is_mutated() {
        let abi = abi_with("function set(string x)");
        let selector = selector_of(&abi, "set");

        let args =
            DynSolValue::Tuple(vec![DynSolValue::String("hello".into())]).abi_encode_params();

        let mut rng = fastrand::Rng::with_seed(42);
        let mut mutator = abi::SequenceArgMutator::new(abi);
        let mut calls = vec![corpus::Call {
            selector,
            args: args.clone(),
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let result = mutator.mutate(&mut rng, &mut calls);
        assert_eq!(result, crate::fuzzer::mutators::MutationResult::Mutated);

        let decoded = DynSolType::Tuple(vec![DynSolType::String])
            .abi_decode_params(&calls[0].args)
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
        let abi = abi_with("function set(function x)");
        let selector = selector_of(&abi, "set");

        let func_val = alloy_primitives::Function::from_slice(&[0u8; 24]);
        let args = DynSolValue::Tuple(vec![DynSolValue::Function(func_val)]).abi_encode_params();

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut calls = vec![corpus::Call {
                selector,
                args: args.clone(),
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }];
            mutator.mutate(&mut rng, &mut calls);
            if calls[0].args != args {
                any_changed = true;
            }
        }
        assert!(any_changed, "function pointer should be mutable");
    }
}
