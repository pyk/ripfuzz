//! Max mode: maximize the `uint256` return value of `max_*` harness functions.
//!
//! In max mode ripfuzz does not check invariants. Instead it executes handler
//! calls followed by every `max_*` function, keeps the highest value and the
//! shortest prefix that produced it, then shrinks each best sequence while
//! preserving its value.

pub use corpus::{MaxBestItem, MaxFuzzerCorpus, MaxShrinkerCorpus};
pub use fuzzer::{MaxFuzzer, MaxFuzzerConfig};
pub use objective::MaxObjective;
pub use output::{MaxFuzzerOutput, MaxResult, MaxShrinkerOutput};
pub use shrinker::{MaxShrinker, MaxShrinkerConfig};

mod corpus;
mod fuzzer;
mod objective;
mod output;
mod shrinker;
