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

use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Result, ensure};

use crate::evm::Transaction;
use crate::max::Call;

/// A sequence of harness calls executed in order.
#[derive(Debug, Clone, Default)]
pub struct Sequence(Vec<Call>);

impl Sequence {
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
}

impl fmt::Display for Sequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let signatures: Vec<String> = self.0.iter().map(|call| call.signature()).collect();
        write!(f, "{}", signatures.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use alloy_json_abi::Function;

    use super::*;

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
}
