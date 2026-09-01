//! Signed changes in the harness value.
//!
//! [`Delta`] is `value_after - value_before` for one handler call, with a
//! sign and a `uint256` magnitude so a dump of `2**255` stays representable.
//!
//! ```rust
//! use alloy_primitives::U256;
//! use ripfuzz::maxer::{Delta, Value};
//!
//! let before = Value::new(U256::from(5008u64));
//! let after = Value::new(U256::from(923u64));
//! let delta = Delta::between(before, after);
//! assert!(delta.is_negative());
//! assert_eq!(delta.magnitude(), U256::from(4085u64));
//! ```

use std::cmp::Ordering;
use std::fmt;

use alloy_primitives::U256;

use crate::maxer::Value;

/// A signed change in the harness value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delta {
    negative: bool,
    magnitude: U256,
}

impl Delta {
    /// Create a delta, normalizing a zero magnitude to a non-negative zero.
    fn new(negative: bool, magnitude: U256) -> Self {
        if magnitude.is_zero() {
            Self {
                negative: false,
                magnitude,
            }
        } else {
            Self {
                negative,
                magnitude,
            }
        }
    }

    /// The delta from `before` to `after`.
    pub fn between(before: Value, after: Value) -> Self {
        if after.get() >= before.get() {
            Self::new(false, after.get() - before.get())
        } else {
            Self::new(true, before.get() - after.get())
        }
    }

    /// A zero delta.
    pub fn zero() -> Self {
        Self::new(false, U256::ZERO)
    }

    /// Whether the value did not change.
    pub fn is_zero(self) -> bool {
        self.magnitude.is_zero()
    }

    /// Whether the value fell.
    pub fn is_negative(self) -> bool {
        self.negative
    }

    /// Whether the value rose.
    pub fn is_positive(self) -> bool {
        !self.negative && !self.magnitude.is_zero()
    }

    /// The absolute change in the harness value.
    pub fn magnitude(self) -> U256 {
        self.magnitude
    }

    /// Corpus sampling weight for this delta.
    ///
    /// Zero deltas are neutral. Non-zero deltas scale with magnitude bit
    /// length so a large dump or gain is drawn more often than a one-wei
    /// nudge, without using the current value as a rank.
    pub fn activity(self) -> u64 {
        if self.is_zero() {
            0
        } else {
            (self.magnitude.bit_len() as u64)
                .saturating_add(1)
                .saturating_mul(8)
        }
    }
}

impl PartialOrd for Delta {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Delta {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.negative, other.negative) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (true, true) => other.magnitude.cmp(&self.magnitude),
            (false, false) => self.magnitude.cmp(&other.magnitude),
        }
    }
}

impl fmt::Display for Delta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_zero() {
            write!(f, "0")
        } else if self.negative {
            write!(f, "-{}", self.magnitude)
        } else {
            write!(f, "+{}", self.magnitude)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(n: u64) -> Value {
        Value::new(U256::from(n))
    }

    #[test]
    fn between_detects_gain_dump_and_flat() {
        let gain = Delta::between(value(10), value(15));
        assert!(gain.is_positive());
        assert_eq!(gain.magnitude(), U256::from(5));

        let dump = Delta::between(value(5008), value(923));
        assert!(dump.is_negative());
        assert_eq!(dump.magnitude(), U256::from(4085));

        let flat = Delta::between(value(923), value(923));
        assert!(flat.is_zero());
        assert!(!flat.is_positive());
        assert!(!flat.is_negative());
    }

    #[test]
    fn ordering_ranks_dumps_below_gains() {
        let dump = Delta::between(value(5008), value(923));
        let small_dump = Delta::between(value(10), value(9));
        let flat = Delta::zero();
        let gain = Delta::between(value(923), value(5035));

        assert!(dump < small_dump);
        assert!(small_dump < flat);
        assert!(flat < gain);
        assert!(dump < gain);
    }

    #[test]
    fn activity_is_zero_only_for_a_flat_call() {
        assert_eq!(Delta::zero().activity(), 0);
        let dump = Delta::between(value(5008), value(923));
        assert!(dump.activity() > 0);
    }

    #[test]
    fn display_includes_the_sign() {
        assert_eq!(Delta::between(value(10), value(15)).to_string(), "+5");
        assert_eq!(Delta::between(value(15), value(10)).to_string(), "-5");
        assert_eq!(Delta::zero().to_string(), "0");
    }
}
