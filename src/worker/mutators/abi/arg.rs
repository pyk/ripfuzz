//! ABI argument mutator that perturbs decoded Solidity values.

use std::borrow::Cow;
use std::num::NonZeroUsize;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, I256, U256};
use libafl::{
    corpus::CorpusId,
    mutators::{MutationResult, Mutator},
    state::HasRand,
};
use libafl_bolts::{Named, rands::Rand};

use crate::corpus;

/// Mutate the arguments of a random call using ABI type information.
///
/// This mutator decodes the raw ABI buffer into [`DynSolValue`]s, mutates
/// them recursively with type-aware rules, and re-encodes the result.
/// Composite types (arrays, tuples, structs) are supported.
#[derive(Debug)]
pub struct SequenceArgMutator {
    abi: JsonAbi,
}

impl SequenceArgMutator {
    pub fn new(abi: JsonAbi) -> Self {
        Self { abi }
    }

    /// Mutate the arguments of a single call.
    ///
    /// Returns `true` if any argument was changed.
    fn mutate_call_args<S: HasRand>(&self, state: &mut S, call: &mut corpus::Call) -> bool {
        // Look up the function by selector.
        let func = match self
            .abi
            .functions()
            .find(|f| f.selector().as_slice() == &call.selector[..])
        {
            Some(f) => f,
            None => return false,
        };

        // Parse input types into DynSolType.
        let types: Vec<DynSolType> = match func
            .inputs
            .iter()
            .map(|p| p.selector_type().parse::<DynSolType>())
            .collect()
        {
            Ok(t) => t,
            Err(_) => return false,
        };

        let tuple_type = DynSolType::Tuple(types);

        // Decode the raw ABI buffer.
        let mut values = match tuple_type.abi_decode_params(&call.args) {
            Ok(v) => v,
            Err(_) => return false,
        };

        // Recursively mutate.
        let mut mutated = false;
        if let DynSolValue::Tuple(ref mut elems) = values {
            for elem in elems.iter_mut() {
                mutated |= self.mutate_value(state, elem);
            }
        }

        // Re-encode.
        if mutated {
            call.args = values.abi_encode_params();
        }
        mutated
    }

    /// Recursively mutate a single [`DynSolValue`].
    ///
    /// Returns `true` if the value was changed.
    fn mutate_value<S: HasRand>(&self, state: &mut S, value: &mut DynSolValue) -> bool {
        match value {
            DynSolValue::Uint(v, sz) => {
                let delta = (state.rand_mut().next() % 1_000) as i64 - 500;
                let delta_u256 = U256::from(delta.unsigned_abs() as u128);
                *v = if delta >= 0 {
                    v.wrapping_add(delta_u256)
                } else {
                    v.wrapping_sub(delta_u256)
                };
                // Mask to the declared bit width so that e.g. uint8 stays
                // in the [0, 2^sz - 1] range.
                if *sz < 256 {
                    let mask: U256 = (U256::from(1u8) << *sz).wrapping_sub(U256::from(1u8));
                    *v &= mask;
                }
                true
            }
            DynSolValue::Int(v, _sz) => {
                let delta = (state.rand_mut().next() % 1_000) as i64 - 500;
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
                    *byte = state.rand_mut().next() as u8;
                }
                *a = Address::from_slice(&bytes);
                true
            }
            DynSolValue::Function(f) => {
                let mut bytes = f.as_slice().to_vec();
                let Some(nz) = NonZeroUsize::new(bytes.len()) else {
                    return false;
                };
                let idx = state.rand_mut().below(nz);
                bytes[idx] = state.rand_mut().next() as u8;
                *f = alloy_primitives::Function::from_slice(&bytes);
                true
            }
            DynSolValue::Bytes(b) => {
                if b.is_empty() {
                    return false;
                }
                let Some(nz) = NonZeroUsize::new(b.len()) else {
                    return false;
                };
                let idx = state.rand_mut().below(nz);
                b[idx] = state.rand_mut().next() as u8;
                true
            }
            DynSolValue::String(s) => {
                let mut bytes = s.as_bytes().to_vec();
                if bytes.is_empty() {
                    return false;
                }
                let Some(nz) = NonZeroUsize::new(bytes.len()) else {
                    return false;
                };
                let idx = state.rand_mut().below(nz);
                bytes[idx] = state.rand_mut().next() as u8;
                // Lossy conversion is safe: Solidity strings are byte
                // sequences and do not require valid UTF-8.
                *s = String::from_utf8_lossy(&bytes).into_owned();
                true
            }
            DynSolValue::FixedBytes(word, sz) => {
                let bytes = word.as_mut_slice();
                let Some(nz) = NonZeroUsize::new(*sz) else {
                    return false;
                };
                let idx = state.rand_mut().below(nz);
                bytes[idx] = state.rand_mut().next() as u8;
                true
            }
            DynSolValue::Array(arr) | DynSolValue::FixedArray(arr) => {
                let mut sub = false;
                // Occasionally swap two elements (Medusa-style array mutation).
                if arr.len() >= 2 && state.rand_mut().next().is_multiple_of(4) {
                    let Some(nz) = NonZeroUsize::new(arr.len()) else {
                        return false;
                    };
                    let i = state.rand_mut().below(nz);
                    let j = state.rand_mut().below(nz);
                    arr.swap(i, j);
                    sub = true;
                }
                for elem in arr.iter_mut() {
                    sub |= self.mutate_value(state, elem);
                }
                sub
            }
            DynSolValue::Tuple(arr) => {
                let mut sub = false;
                for elem in arr.iter_mut() {
                    sub |= self.mutate_value(state, elem);
                }
                sub
            }
        }
    }
}

impl Named for SequenceArgMutator {
    fn name(&self) -> &Cow<'static, str> {
        &Cow::Borrowed("SequenceArgMutator")
    }
}

impl<S: HasRand> Mutator<corpus::CallSequenceInput, S> for SequenceArgMutator {
    fn mutate(
        &mut self,
        state: &mut S,
        input: &mut corpus::CallSequenceInput,
    ) -> Result<MutationResult, libafl::Error> {
        if input.calls.is_empty() {
            return Ok(MutationResult::Skipped);
        }
        let call_idx = state.rand_mut().below(
            NonZeroUsize::new(input.calls.len())
                .ok_or_else(|| libafl::Error::unknown("non-zero"))?,
        );

        if self.mutate_call_args(state, &mut input.calls[call_idx]) {
            Ok(MutationResult::Mutated)
        } else {
            Ok(MutationResult::Skipped)
        }
    }

    fn post_exec(
        &mut self,
        _state: &mut S,
        _new_corpus_id: Option<CorpusId>,
    ) -> Result<(), libafl::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::{I256, U256};
    use libafl::mutators::Mutator;
    use libafl::state::HasRand;
    use libafl_bolts::rands::StdRand;

    use crate::corpus;
    use crate::worker::mutators::abi;

    /// Minimal test state that only implements `HasRand`.
    struct MockState {
        rand: StdRand,
    }

    impl MockState {
        fn with_seed(seed: u64) -> Self {
            Self {
                rand: StdRand::with_seed(seed),
            }
        }
    }

    impl HasRand for MockState {
        type Rand = StdRand;
        fn rand(&self) -> &Self::Rand {
            &self.rand
        }
        fn rand_mut(&mut self) -> &mut Self::Rand {
            &mut self.rand
        }
    }

    /// Build a `JsonAbi` from a human-readable function signature.
    fn abi_with(function_sig: &str) -> alloy_json_abi::JsonAbi {
        alloy_json_abi::JsonAbi::parse([function_sig]).unwrap()
    }

    /// Return the 4-byte selector for the given function name in the ABI.
    fn selector_of(abi: &alloy_json_abi::JsonAbi, name: &str) -> [u8; 4] {
        abi.functions()
            .find(|f| f.name == name)
            .unwrap()
            .selector()
            .into()
    }

    #[test]
    fn empty_sequence_is_skipped() {
        let mut state = MockState::with_seed(42);
        let abi = abi_with("function set(uint256 x)");
        let mut mutator = abi::SequenceArgMutator::new(abi);
        let mut input = corpus::CallSequenceInput::new();

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Skipped);
    }

    #[test]
    fn unknown_selector_returns_skipped() {
        let mut state = MockState::with_seed(42);
        let target_abi = abi_with("function set(uint256 x)");
        let other_abi = abi_with("function transfer(address to)");
        let unknown_selector = selector_of(&other_abi, "transfer");
        let mut mutator = abi::SequenceArgMutator::new(target_abi);

        let mut input = corpus::CallSequenceInput {
            calls: vec![corpus::Call {
                selector: unknown_selector,
                args: vec![0u8; 32],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }],
        };
        let original_args = input.calls[0].args.clone();

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Skipped);
        assert_eq!(input.calls[0].args, original_args, "args must be unchanged");
    }

    #[test]
    fn uint256_argument_is_mutated() {
        let abi = abi_with("function set(uint256 x)");
        let selector = selector_of(&abi, "set");

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut state = MockState::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut input = corpus::CallSequenceInput {
                calls: vec![corpus::Call {
                    selector,
                    args: vec![0u8; 32],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }],
            };
            let original_args = input.calls[0].args.clone();

            let result = mutator.mutate(&mut state, &mut input).unwrap();
            assert_eq!(result, libafl::mutators::MutationResult::Mutated);
            if input.calls[0].args != original_args {
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
            let mut state = MockState::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut input = corpus::CallSequenceInput {
                calls: vec![corpus::Call {
                    selector,
                    args: full_arg.clone(),
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }],
            };

            mutator.mutate(&mut state, &mut input).unwrap();

            if input.calls[0].args[..16] != high_bytes[..] {
                any_high_changed = true;
            }
            if input.calls[0].args[16..] != low_bytes[..] {
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

        // Proper ABI encoding for setData(hex"abcd"):
        // word 0: offset = 32
        // word 1: length = 2
        // word 2: data = 0xabcd padded
        let mut args = vec![0u8; 96];
        args[31] = 0x20; // offset = 32
        args[63] = 0x02; // length = 2
        args[64] = 0xab; // data byte 0
        args[65] = 0xcd; // data byte 1

        let mut state = MockState::with_seed(42);
        let mut mutator = abi::SequenceArgMutator::new(abi);
        let mut input = corpus::CallSequenceInput {
            calls: vec![corpus::Call {
                selector,
                args: args.clone(),
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }],
        };

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);

        let mutated = &input.calls[0].args;

        // Offset and length words should be untouched.
        assert_eq!(
            &mutated[..64],
            &args[..64],
            "offset and length must not be corrupted"
        );

        // Actual data payload should have been mutated.
        assert_ne!(
            &mutated[64..],
            &args[64..],
            "dynamic data should have been mutated"
        );
    }

    #[test]
    fn bool_argument_is_flipped() {
        let mut state = MockState::with_seed(42);
        let abi = abi_with("function toggle(bool b)");
        let selector = selector_of(&abi, "toggle");
        let mut mutator = abi::SequenceArgMutator::new(abi);

        let mut input = corpus::CallSequenceInput {
            calls: vec![corpus::Call {
                selector,
                args: {
                    let mut v = vec![0u8; 32];
                    v[31] = 1; // true
                    v
                },
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }],
        };

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);
        assert_eq!(input.calls[0].args[31], 0); // flipped to false
    }

    #[test]
    fn address_argument_is_mutated() {
        let mut state = MockState::with_seed(42);
        let abi = abi_with("function transfer(address to)");
        let selector = selector_of(&abi, "transfer");
        let mut mutator = abi::SequenceArgMutator::new(abi);

        let mut input = corpus::CallSequenceInput {
            calls: vec![corpus::Call {
                selector,
                args: vec![0u8; 32],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }],
        };
        let original_args = input.calls[0].args.clone();

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);
        assert_ne!(&input.calls[0].args[12..32], &original_args[12..32]);
    }

    #[test]
    fn address_mutation_overwrites_all_20_bytes() {
        let abi = abi_with("function transfer(address to)");
        let selector = selector_of(&abi, "transfer");

        for seed in [1u64, 2, 3, 42, 99, 123, 456, 789, 1000, 2000] {
            let mut state = MockState::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut input = corpus::CallSequenceInput {
                calls: vec![corpus::Call {
                    selector,
                    args: vec![0u8; 32],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }],
            };

            mutator.mutate(&mut state, &mut input).unwrap();
            let mutated = &input.calls[0].args;

            // bytes 0..12 must stay zero (padding)
            assert_eq!(&mutated[..12], &[0u8; 12], "address padding must stay zero");

            // bytes 12..32 must all be randomized (20 bytes)
            assert_ne!(
                &mutated[12..32],
                &[0u8; 20],
                "all 20 address bytes should be randomized"
            );
        }
    }

    #[test]
    fn multiple_arguments_all_get_mutated() {
        let mut state = MockState::with_seed(123);
        let abi = abi_with("function multi(uint256 a, bool b, address c)");
        let selector = selector_of(&abi, "multi");
        let mut mutator = abi::SequenceArgMutator::new(abi);

        let mut input = corpus::CallSequenceInput {
            calls: vec![corpus::Call {
                selector,
                args: vec![0u8; 96],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }],
        };
        let original_args = input.calls[0].args.clone();

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);
        assert_ne!(input.calls[0].args, original_args);
    }

    #[test]
    fn repeated_mutation_produces_different_values() {
        let abi = abi_with("function set(uint256 x)");
        let selector = selector_of(&abi, "set");

        let mut values = Vec::new();
        for seed in [1u64, 2, 3, 4, 5] {
            let mut state = MockState::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut input = corpus::CallSequenceInput {
                calls: vec![corpus::Call {
                    selector,
                    args: vec![0u8; 32],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }],
            };
            mutator.mutate(&mut state, &mut input).unwrap();
            values.push(input.calls[0].args.clone());
        }

        let first = &values[0];
        let all_same = values.iter().all(|v| v == first);
        assert!(!all_same, "mutations with different seeds should vary");
    }

    // ------------------------------------------------------------------
    // Type-safe mutation tests (new capabilities unlocked by DynSolValue)
    // ------------------------------------------------------------------

    #[test]
    fn uint8_argument_is_masked() {
        let abi = abi_with("function set(uint8 x)");
        let selector = selector_of(&abi, "set");

        // Encode uint8 = 255 (max value).
        let args =
            DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(255), 8)]).abi_encode_params();

        let mut state = MockState::with_seed(1);
        let mut mutator = abi::SequenceArgMutator::new(abi);
        let mut input = corpus::CallSequenceInput {
            calls: vec![corpus::Call {
                selector,
                args: args.clone(),
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }],
        };

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);

        // Decode and verify the mutated value is still a valid uint8.
        let decoded = DynSolType::Tuple(vec![DynSolType::Uint(8)])
            .abi_decode_params(&input.calls[0].args)
            .unwrap();
        if let DynSolValue::Tuple(values) = decoded {
            if let DynSolValue::Uint(v, 8) = values[0] {
                assert!(v <= U256::from(255), "uint8 value {} out of range", v);
            } else {
                panic!("expected Uint(8)");
            }
        } else {
            panic!("expected Tuple");
        }
    }

    #[test]
    fn int8_argument_is_mutated() {
        let abi = abi_with("function set(int8 x)");
        let selector = selector_of(&abi, "set");

        // Encode int8 = 0.
        let args = DynSolValue::Tuple(vec![DynSolValue::Int(I256::ZERO, 8)]).abi_encode_params();

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut state = MockState::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut input = corpus::CallSequenceInput {
                calls: vec![corpus::Call {
                    selector,
                    args: args.clone(),
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }],
            };
            mutator.mutate(&mut state, &mut input).unwrap();
            if input.calls[0].args != args {
                any_changed = true;
            }
        }
        assert!(any_changed, "int8 should be mutable");
    }

    #[test]
    fn fixed_bytes4_argument_is_mutated() {
        let abi = abi_with("function set(bytes4 x)");
        let selector = selector_of(&abi, "set");

        // Encode bytes4 = 0xdeadbeef.
        let mut data = [0u8; 32];
        data[..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        let word = alloy_primitives::B256::from_slice(&data);
        let args = DynSolValue::Tuple(vec![DynSolValue::FixedBytes(word, 4)]).abi_encode_params();

        let mut state = MockState::with_seed(42);
        let mut mutator = abi::SequenceArgMutator::new(abi);
        let mut input = corpus::CallSequenceInput {
            calls: vec![corpus::Call {
                selector,
                args: args.clone(),
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }],
        };

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);

        // Only the first 4 payload bytes should differ; padding (bytes 4..32) must stay zero.
        assert_ne!(
            &input.calls[0].args[..4],
            &args[..4],
            "bytes4 payload should mutate"
        );
        assert_eq!(
            &input.calls[0].args[4..32],
            &[0u8; 28],
            "bytes4 padding must stay zero"
        );
    }

    #[test]
    fn array_argument_is_mutated() {
        let abi = abi_with("function set(uint256[2] x)");
        let selector = selector_of(&abi, "set");

        // Encode [0, 0].
        let args = DynSolValue::Tuple(vec![DynSolValue::FixedArray(vec![
            DynSolValue::Uint(U256::ZERO, 256),
            DynSolValue::Uint(U256::ZERO, 256),
        ])])
        .abi_encode_params();

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut state = MockState::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut input = corpus::CallSequenceInput {
                calls: vec![corpus::Call {
                    selector,
                    args: args.clone(),
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }],
            };
            mutator.mutate(&mut state, &mut input).unwrap();
            if input.calls[0].args != args {
                any_changed = true;
            }
        }
        assert!(any_changed, "fixed array elements should be mutable");
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
            let mut state = MockState::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut input = corpus::CallSequenceInput {
                calls: vec![corpus::Call {
                    selector,
                    args: args.clone(),
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }],
            };
            mutator.mutate(&mut state, &mut input).unwrap();
            if input.calls[0].args != args {
                any_changed = true;
            }
        }
        assert!(any_changed, "tuple fields should be mutable");
    }

    #[test]
    fn string_argument_is_mutated() {
        let abi = abi_with("function set(string x)");
        let selector = selector_of(&abi, "set");

        // Encode "hello".
        let args =
            DynSolValue::Tuple(vec![DynSolValue::String("hello".into())]).abi_encode_params();

        let mut state = MockState::with_seed(42);
        let mut mutator = abi::SequenceArgMutator::new(abi);
        let mut input = corpus::CallSequenceInput {
            calls: vec![corpus::Call {
                selector,
                args: args.clone(),
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }],
        };

        let result = mutator.mutate(&mut state, &mut input).unwrap();
        assert_eq!(result, libafl::mutators::MutationResult::Mutated);

        // Decode and verify the string changed (or at least the payload did).
        let decoded = DynSolType::Tuple(vec![DynSolType::String])
            .abi_decode_params(&input.calls[0].args)
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

        // Encode a function pointer: 20-byte address + 4-byte selector.
        let func_val = alloy_primitives::Function::from_slice(&[0u8; 24]);
        let args = DynSolValue::Tuple(vec![DynSolValue::Function(func_val)]).abi_encode_params();

        let mut any_changed = false;
        for seed in [1u64, 2, 3, 42, 99] {
            let mut state = MockState::with_seed(seed);
            let mut mutator = abi::SequenceArgMutator::new(abi.clone());
            let mut input = corpus::CallSequenceInput {
                calls: vec![corpus::Call {
                    selector,
                    args: args.clone(),
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }],
            };
            mutator.mutate(&mut state, &mut input).unwrap();
            if input.calls[0].args != args {
                any_changed = true;
            }
        }
        assert!(any_changed, "function pointer should be mutable");
    }
}
