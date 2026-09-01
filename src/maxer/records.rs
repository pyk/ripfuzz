//! Per-class min and max value-delta records.
//!
//! [`Records`] keep one minimum and one maximum [`Delta`] per
//! `(prefix selectors, handler)` class. A new extreme, or a climb out of a
//! dip, is novelty the fuzzer should keep exploring from.
//!
//! ```rust
//! use alloy_json_abi::Function;
//! use alloy_primitives::U256;
//! use ripfuzz::maxer::{Call, Delta, Records, Sequence, Value};
//!
//! use alloy_dyn_abi::DynSolValue;
//!
//! let records = Records::new();
//! let reduce = Call::new(
//!     Function::parse("reduce()").unwrap(),
//!     DynSolValue::Tuple(vec![]),
//! );
//! let delta = Delta::between(Value::new(U256::from(5008u64)), Value::new(U256::from(923u64)));
//! let observation = records.observe(&Sequence::empty(), &reduce, delta, false);
//! assert!(observation.new_record_min);
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alloy_primitives::Selector;

use crate::maxer::{Call, Delta, Sequence};

/// Result of observing one call's delta against the campaign records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Observation {
    /// Whether this delta is the largest dump seen for its class.
    pub new_record_min: bool,
    /// Whether this delta is the largest gain seen for its class.
    pub new_record_max: bool,
    /// Whether the call climbed while the trajectory was below baseline.
    pub recovery: bool,
}

impl Observation {
    /// Whether any delta signal should admit the prefix.
    pub fn is_interesting(self) -> bool {
        self.new_record_min || self.new_record_max || self.recovery
    }
}

/// One class of `(prefix signature, final handler)`.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct Class {
    prefix: Vec<Selector>,
    handler: Selector,
}

/// The extreme deltas recorded for one class.
#[derive(Clone, Copy, Debug)]
struct ClassRecord {
    min: Delta,
    max: Delta,
}

/// Campaign-wide min and max delta records, shared across fuzzers.
///
/// Cloning is cheap (shares the same inner map). Records reset between
/// campaigns because they are never persisted.
#[derive(Debug, Clone)]
pub struct Records {
    inner: Arc<Mutex<HashMap<Class, ClassRecord>>>,
}

impl Records {
    /// Create empty records.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Lock the map, recovering from poisoning.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Class, ClassRecord>> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Observe one call's delta and return the novelty flags.
    ///
    /// `prefix` is the sequence of handlers before this call. Zero deltas are
    /// neutral: they neither update records nor set recovery.
    ///
    /// `below_baseline` is whether the value before this call was strictly
    /// below the initial harness value.
    pub fn observe(
        &self,
        prefix: &Sequence,
        handler: &Call,
        delta: Delta,
        below_baseline: bool,
    ) -> Observation {
        // 1. Zero deltas are neither records nor recoveries.
        if delta.is_zero() {
            return Observation::default();
        }

        // 2. A climb out of a dip is always a recovery, independent of records.
        let recovery = below_baseline && delta.is_positive();

        // 3. Compare against the one min and one max stored for this class.
        let class = Class {
            prefix: prefix.calls().iter().map(Call::selector).collect(),
            handler: handler.selector(),
        };
        let mut records = self.lock();
        match records.get_mut(&class) {
            None => {
                records.insert(
                    class,
                    ClassRecord {
                        min: delta,
                        max: delta,
                    },
                );
                Observation {
                    new_record_min: delta.is_negative(),
                    new_record_max: delta.is_positive(),
                    recovery,
                }
            }
            Some(record) => {
                let new_record_min = delta.is_negative() && delta < record.min;
                let new_record_max = delta.is_positive() && delta > record.max;
                if delta < record.min {
                    record.min = delta;
                }
                if delta > record.max {
                    record.max = delta;
                }
                Observation {
                    new_record_min,
                    new_record_max,
                    recovery,
                }
            }
        }
    }
}

impl Default for Records {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;
    use alloy_primitives::U256;

    use super::*;
    use crate::maxer::{Call, Value};

    fn call(name: &str) -> Call {
        Call::new(Function::parse(name).unwrap(), DynSolValue::Tuple(vec![]))
    }

    fn dump() -> Delta {
        Delta::between(
            Value::new(U256::from(5008u64)),
            Value::new(U256::from(923u64)),
        )
    }

    fn gain() -> Delta {
        Delta::between(
            Value::new(U256::from(923u64)),
            Value::new(U256::from(5035u64)),
        )
    }

    #[test]
    fn first_dump_is_a_min_record_not_a_max() {
        let records = Records::new();
        let observation = records.observe(&Sequence::empty(), &call("reduce()"), dump(), false);

        assert!(observation.new_record_min);
        assert!(!observation.new_record_max);
        assert!(!observation.recovery);
        assert!(observation.is_interesting());
    }

    #[test]
    fn first_gain_is_a_max_record_not_a_min() {
        let records = Records::new();
        let observation = records.observe(&Sequence::empty(), &call("increase()"), gain(), true);

        assert!(!observation.new_record_min);
        assert!(observation.new_record_max);
        assert!(observation.recovery);
    }

    #[test]
    fn a_larger_dump_of_the_same_class_is_a_new_min() {
        let records = Records::new();
        let reduce = call("reduce()");
        let small = Delta::between(Value::new(U256::from(10u64)), Value::new(U256::from(9u64)));
        records.observe(&Sequence::empty(), &reduce, small, false);

        let observation = records.observe(&Sequence::empty(), &reduce, dump(), false);
        assert!(observation.new_record_min);
        assert!(!observation.new_record_max);
    }

    #[test]
    fn a_repeat_dump_is_not_a_new_record() {
        let records = Records::new();
        let reduce = call("reduce()");
        records.observe(&Sequence::empty(), &reduce, dump(), false);

        let observation = records.observe(&Sequence::empty(), &reduce, dump(), false);
        assert!(!observation.is_interesting());
    }

    #[test]
    fn zero_delta_is_neutral() {
        let records = Records::new();
        let observation = records.observe(&Sequence::empty(), &call("swap()"), Delta::zero(), true);

        assert_eq!(observation, Observation::default());
    }

    #[test]
    fn classes_track_records_independently() {
        let records = Records::new();
        let reduce = call("reduce()");
        let swap = call("swap()");
        records.observe(&Sequence::empty(), &reduce, dump(), false);

        let observation = records.observe(&Sequence::new(vec![reduce]), &swap, dump(), true);
        assert!(observation.new_record_min);
    }
}
