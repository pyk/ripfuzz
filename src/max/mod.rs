//! Max-mode shrinking: minimize the best `max_*` result while preserving its
//! value.
//!
//! The maxxing fuzzer records the highest `max_*` return value and the
//! shortest prefix that produced it (see [`crate::fuzzers::MaxxingFuzzer`]);
//! this module shrinks that best sequence while keeping its value.

pub use corpus::MaxShrinkerCorpus;
pub use output::{MaxResult, MaxShrinkerOutput};
pub use shrinker::{MaxShrinker, MaxShrinkerConfig};

mod corpus;
mod output;
mod shrinker;
