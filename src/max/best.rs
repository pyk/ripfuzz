//! The best sequence found by the fuzzer.
//!
//! [`Best`] tracks the highest-value sequence seen so far, seeded with the
//! empty sequence and the initial harness value.
//!
//! ```rust
//! use ripfuzz::max::{Best, Sequence, Value};
//!
//! // let mut best = Best::new(Sequence::empty(), initial_value);
//! // if best.consider(sequence, value, chain) { /* new maximum */ }
//! ```

use crate::evm::Chain;
use crate::max::{Sequence, Value};

/// The highest-value sequence found by the fuzzer, with the state after
/// executing it so the fuzzer can extend the best state directly.
#[derive(Debug, Clone)]
pub struct Best {
    value: Value,
    sequence: Sequence,
    chain: Option<Chain>,
}

impl Best {
    /// Create a best result from a seed sequence and its value.
    pub fn new(sequence: Sequence, value: Value) -> Self {
        Self {
            value,
            sequence,
            chain: None,
        }
    }

    /// Create a best result that also carries the state after its sequence.
    pub fn with_chain(sequence: Sequence, value: Value, chain: Chain) -> Self {
        Self {
            value,
            sequence,
            chain: Some(chain),
        }
    }

    /// The final value of the best sequence.
    pub fn value(&self) -> Value {
        self.value
    }

    /// The best sequence.
    pub fn sequence(&self) -> &Sequence {
        &self.sequence
    }

    /// The state after executing the best sequence, when known.
    pub fn chain(&self) -> Option<&Chain> {
        self.chain.as_ref()
    }

    /// Record the sequence when it is better than the current best,
    /// returning whether it replaced it.
    ///
    /// Better means a strictly higher value, or the same value with fewer
    /// calls once a sequence has improved the initial value. The tie-break
    /// frees call slots occupied by calls that do not affect the value, so
    /// mutations can extend the value further within the call limit.
    pub fn consider(&mut self, sequence: Sequence, value: Value, chain: Chain) -> bool {
        let shorter = value == self.value
            && !self.sequence.is_empty()
            && sequence.len() < self.sequence.len();
        if value > self.value || shorter {
            self.value = value;
            self.sequence = sequence;
            self.chain = Some(chain);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_json_abi::Function;
    use alloy_primitives::U256;
    use revm::primitives::Bytes;

    use super::*;
    use crate::evm::{ChainConfig, TransactionResult};

    fn value(n: u64) -> Value {
        Value::decode(&success_result(n)).unwrap()
    }

    fn success_result(n: u64) -> TransactionResult {
        let mut output = vec![0; 32];
        output[31] = n as u8;
        TransactionResult {
            success: true,
            gas_used: 0,
            output: Some(Bytes::from(output)),
            ..Default::default()
        }
    }

    #[test]
    fn consider_replaces_only_higher_values() {
        let mut best = Best::new(Sequence::empty(), value(10));

        assert!(!best.consider(
            Sequence::empty(),
            value(9),
            Chain::empty(ChainConfig::default())
        ));
        assert_eq!(best.value(), value(10));

        assert!(best.consider(
            Sequence::empty(),
            value(11),
            Chain::empty(ChainConfig::default())
        ));
        assert_eq!(best.value(), value(11));

        assert!(!best.consider(
            Sequence::empty(),
            value(11),
            Chain::empty(ChainConfig::default())
        ));
        assert_eq!(best.value(), value(11));
    }

    #[test]
    fn consider_keeps_the_replacing_sequence() {
        let mut best = Best::new(Sequence::empty(), value(10));
        let mut rng = fastrand::Rng::new();
        let sequence = Sequence::random(&mut rng, &[Function::parse("inc()").unwrap()], 1).unwrap();

        assert!(best.consider(
            sequence.clone(),
            value(12),
            Chain::empty(ChainConfig::default())
        ));
        assert_eq!(best.sequence().len(), sequence.len());
        assert_eq!(U256::from(best.value().get()), U256::from(12));
    }

    #[test]
    fn consider_replaces_equal_value_with_fewer_calls() {
        let mut rng = fastrand::Rng::new();
        let long = sequence_of_len(&mut rng, 4);
        let short = sequence_of_len(&mut rng, 2);
        let mut best = Best::new(long, value(10));

        assert!(best.consider(short, value(10), Chain::empty(ChainConfig::default())));
        assert_eq!(best.sequence().len(), 2);
        assert_eq!(best.value(), value(10));
    }

    #[test]
    fn consider_never_ties_against_the_empty_sequence() {
        let mut rng = fastrand::Rng::new();
        let sequence = sequence_of_len(&mut rng, 2);
        let mut best = Best::new(Sequence::empty(), value(10));

        assert!(!best.consider(sequence, value(10), Chain::empty(ChainConfig::default())));
        assert!(best.sequence().is_empty());
    }

    #[test]
    fn consider_keeps_equal_value_with_equal_calls() {
        let mut rng = fastrand::Rng::new();
        let first = sequence_of_len(&mut rng, 2);
        let second = sequence_of_len(&mut rng, 2);
        let mut best = Best::new(first.clone(), value(10));

        assert!(!best.consider(second, value(10), Chain::empty(ChainConfig::default())));
        assert_eq!(best.sequence().len(), first.len());
    }

    #[test]
    fn consider_stores_the_chain_of_the_replacing_sequence() {
        let mut best = Best::new(Sequence::empty(), value(10));
        assert!(best.chain().is_none());

        let sequence = Sequence::empty();
        let chain = Chain::empty(ChainConfig::default());
        assert!(best.consider(sequence, value(11), chain));
        assert!(best.chain().is_some());
    }

    #[test]
    fn consider_tie_keeps_the_existing_chain() {
        let mut best = Best::new(Sequence::empty(), value(10));
        let seed_chain = Chain::empty(ChainConfig::default());
        assert!(!best.consider(Sequence::empty(), value(10), seed_chain));
        assert!(best.chain().is_none());
    }

    #[test]
    fn shorter_sequence_replaces_with_its_own_chain() {
        let mut rng = fastrand::Rng::new();
        let long = sequence_of_len(&mut rng, 4);
        let short = sequence_of_len(&mut rng, 2);
        let mut best = Best::new(long, value(10));
        assert!(best.consider(short, value(10), Chain::empty(ChainConfig::default())));
        assert_eq!(best.sequence().len(), 2);
        assert!(best.chain().is_some());
    }

    /// A random sequence with exactly `len` calls.
    ///
    /// `Sequence::random` draws the length from `1..=max_calls`, so keep
    /// drawing until the requested length comes up.
    fn sequence_of_len(rng: &mut fastrand::Rng, len: usize) -> Sequence {
        let handlers = [Function::parse("inc()").unwrap()];
        loop {
            let sequence = Sequence::random(rng, &handlers, len).unwrap();
            if sequence.len() == len {
                return sequence;
            }
        }
    }
}
