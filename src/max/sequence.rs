//! Sequences of harness calls.
//!
//! [`Sequence`] is the fuzzed unit: a list of handler calls executed in order
//! against the deployed harness.
//!
//! ```rust
//! use alloy_json_abi::Function;
//! use ripfuzz::max::Sequence;
//!
//! // let handlers: Vec<Function> = max_harness.handlers()...;
//! // let sequence = Sequence::random(&mut rng, &handlers, 8)?;
//! ```

use std::fmt;
use std::ops::Range;

use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Result, ensure};

use crate::evm::Transaction;
use crate::max::Call;

/// A sequence of harness calls executed in order.
#[derive(Debug, Clone, Default)]
pub struct Sequence(Vec<Call>);

impl Sequence {
    /// Create a sequence from its calls.
    pub fn new(calls: Vec<Call>) -> Self {
        Self(calls)
    }

    /// Create an empty sequence.
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    /// Generate a random sequence of calls from the handler functions.
    pub fn random(
        rng: &mut fastrand::Rng,
        handlers: &[Function],
        max_calls: usize,
    ) -> Result<Self> {
        ensure!(
            !handlers.is_empty(),
            "no handler functions to generate calls from"
        );
        let len = rng.usize(1..=max_calls.max(1));
        let mut calls = Vec::with_capacity(len);
        for _ in 0..len {
            let function = &handlers[rng.usize(..handlers.len())];
            calls.push(Call::random(rng, function)?);
        }
        Ok(Self(calls))
    }

    /// The calls in the sequence.
    pub fn calls(&self) -> &[Call] {
        &self.0
    }

    /// The number of calls in the sequence.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the sequence has no calls.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Build the transactions for this sequence against the target.
    pub fn transactions(&self, target: Address, caller: Address) -> Vec<Transaction> {
        self.0
            .iter()
            .map(|call| call.transaction(target, caller))
            .collect()
    }

    /// Create a copy of this sequence without the calls in the given range.
    pub fn without(&self, range: Range<usize>) -> Sequence {
        let mut calls = self.0.clone();
        calls.drain(range);
        Self(calls)
    }

    /// Mutate this sequence with a random operation.
    ///
    /// Operations: insert, delete, replace, duplicate, splice with `other`,
    /// and argument regeneration. When an operation cannot apply or the
    /// result would be empty, a fresh random sequence is generated instead.
    pub fn mutate(
        &self,
        rng: &mut fastrand::Rng,
        handlers: &[Function],
        other: &Sequence,
        max_calls: usize,
    ) -> Result<Self> {
        ensure!(
            !handlers.is_empty(),
            "no handler functions to mutate calls from"
        );
        if self.0.is_empty() {
            return Self::random(rng, handlers, max_calls);
        }
        let mut calls = self.0.clone();
        match rng.usize(..6) {
            // insert a fresh call at a random position
            0 => {
                let function = &handlers[rng.usize(..handlers.len())];
                let call = Call::random(rng, function)?;
                let pos = rng.usize(..=calls.len());
                calls.insert(pos, call);
            }
            // delete a call
            1 if calls.len() > 1 => {
                let pos = rng.usize(..calls.len());
                calls.remove(pos);
            }
            // replace a call with a fresh one
            2 => {
                let function = &handlers[rng.usize(..handlers.len())];
                let call = Call::random(rng, function)?;
                let pos = rng.usize(..calls.len());
                calls[pos] = call;
            }
            // duplicate a call right after itself
            3 => {
                let pos = rng.usize(..calls.len());
                let call = calls[pos].clone();
                calls.insert(pos + 1, call);
            }
            // splice: prefix of this sequence with a suffix of the other
            4 if !other.0.is_empty() => {
                let pos = rng.usize(..=calls.len());
                let suffix_start = pos.min(other.0.len());
                calls.truncate(pos);
                calls.extend_from_slice(&other.0[suffix_start..]);
            }
            // regenerate the arguments of one call
            _ => {
                let pos = rng.usize(..calls.len());
                let function = calls[pos].function().clone();
                calls[pos] = Call::random(rng, &function)?;
            }
        }
        if calls.is_empty() {
            return Self::random(rng, handlers, max_calls);
        }
        calls.truncate(max_calls);
        Ok(Self(calls))
    }
}

impl fmt::Display for Sequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let signatures: Vec<String> = self.0.iter().map(|call| call.signature()).collect();
        write!(f, "{}", signatures.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;

    use super::*;

    fn call(name: &str) -> Call {
        Call::new(Function::parse(name).unwrap(), DynSolValue::Tuple(vec![]))
    }

    fn handlers() -> Vec<Function> {
        vec![
            Function::parse("inc()").unwrap(),
            Function::parse("set(uint256)").unwrap(),
        ]
    }

    #[test]
    fn random_length_within_bounds() {
        let mut rng = fastrand::Rng::new();
        for _ in 0..32 {
            let sequence = Sequence::random(&mut rng, &handlers(), 3).unwrap();
            assert!((1..=3).contains(&sequence.len()));
        }
    }

    #[test]
    fn random_calls_use_handlers() {
        let mut rng = fastrand::Rng::new();
        let sequence = Sequence::random(&mut rng, &handlers(), 4).unwrap();

        for call in &sequence.0 {
            let signature = call.signature();
            assert!(
                signature == "inc()" || signature == "set(uint256)",
                "unexpected call signature: {signature}"
            );
        }
    }

    #[test]
    fn random_with_empty_handlers_fails() {
        let mut rng = fastrand::Rng::new();
        let err = Sequence::random(&mut rng, &[], 4).unwrap_err();

        assert_eq!(
            err.to_string(),
            "no handler functions to generate calls from"
        );
    }

    #[test]
    fn empty_sequence_builds_no_transactions() {
        let sequence = Sequence::empty();

        assert!(sequence.is_empty());
        assert!(
            sequence
                .transactions(Address::ZERO, Address::ZERO)
                .is_empty()
        );
    }

    #[test]
    fn display_joins_signatures() {
        let mut rng = fastrand::Rng::new();
        let sequence = Sequence::random(&mut rng, &handlers(), 1).unwrap();
        let displayed = sequence.to_string();

        assert_eq!(displayed, sequence.0[0].signature());
    }

    #[test]
    fn without_removes_the_range() {
        let sequence = Sequence(vec![call("a()"), call("b()"), call("c()")]);

        let without = sequence.without(1..3);
        assert_eq!(without.len(), 1);
        assert_eq!(without.0[0].signature(), "a()");

        let without_all = sequence.without(0..3);
        assert!(without_all.is_empty());
    }

    #[test]
    fn mutate_respects_the_call_limit() {
        let mut rng = fastrand::Rng::new();
        for _ in 0..32 {
            let base = Sequence::random(&mut rng, &handlers(), 8).unwrap();
            let mutated = base
                .mutate(&mut rng, &handlers(), &Sequence::empty(), 4)
                .unwrap();
            assert!((1..=4).contains(&mutated.len()));
        }
    }

    #[test]
    fn mutate_an_empty_sequence_generates_a_random_one() {
        let mut rng = fastrand::Rng::new();
        let mutated = Sequence::empty()
            .mutate(&mut rng, &handlers(), &Sequence::empty(), 4)
            .unwrap();

        assert!(!mutated.is_empty());
    }

    #[test]
    fn mutate_with_an_empty_other_still_works() {
        let mut rng = fastrand::Rng::new();
        let base = Sequence::random(&mut rng, &handlers(), 4).unwrap();
        for _ in 0..32 {
            let mutated = base
                .mutate(&mut rng, &handlers(), &Sequence::empty(), 4)
                .unwrap();
            assert!(!mutated.is_empty());
        }
    }
}
