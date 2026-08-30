//! The best sequence found by the fuzzer.
//!
//! [`Best`] tracks the highest-value sequence seen so far, seeded with the
//! empty sequence and the initial harness value.
//!
//! ```rust
//! use ripfuzz::max::{Best, Sequence, Value};
//!
//! // let mut best = Best::new(Sequence::empty(), initial_value);
//! // if best.consider(sequence, value) { /* new maximum */ }
//! ```

use crate::max::{Sequence, Value};

/// The highest-value sequence found by the fuzzer.
#[derive(Debug, Clone)]
pub struct Best {
    value: Value,
    sequence: Sequence,
}

impl Best {
    /// Create a best result from a seed sequence and its value.
    pub fn new(sequence: Sequence, value: Value) -> Self {
        Self { value, sequence }
    }

    /// The final value of the best sequence.
    pub fn value(&self) -> Value {
        self.value
    }

    /// The best sequence.
    pub fn sequence(&self) -> &Sequence {
        &self.sequence
    }

    /// Record the sequence when its value is strictly higher, returning
    /// whether it replaced the current best.
    pub fn consider(&mut self, sequence: Sequence, value: Value) -> bool {
        if value > self.value {
            self.value = value;
            self.sequence = sequence;
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
    use crate::evm::TransactionResult;

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

        assert!(!best.consider(Sequence::empty(), value(9)));
        assert_eq!(best.value(), value(10));

        assert!(best.consider(Sequence::empty(), value(11)));
        assert_eq!(best.value(), value(11));

        assert!(!best.consider(Sequence::empty(), value(11)));
        assert_eq!(best.value(), value(11));
    }

    #[test]
    fn consider_keeps_the_replacing_sequence() {
        let mut best = Best::new(Sequence::empty(), value(10));
        let mut rng = fastrand::Rng::new();
        let sequence = Sequence::random(&mut rng, &[Function::parse("inc()").unwrap()], 1).unwrap();

        assert!(best.consider(sequence.clone(), value(12)));
        assert_eq!(best.sequence().len(), sequence.len());
        assert_eq!(U256::from(best.value().get()), U256::from(12));
    }
}
