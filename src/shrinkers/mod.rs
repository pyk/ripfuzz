//! Per-thread shrinkers for invariant and maxxing campaigns.
//!
//! [`InvariantShrinker`] minimizes a failing corpus item while it still
//! triggers a failed assertion. [`MaxxingShrinker`] minimizes the best
//! `max_*` sequence while preserving its value.

pub use crate::shrinkers::corpus::MaxxingShrinkerCorpus;
pub use crate::shrinkers::invariant::{InvariantShrinker, InvariantShrinkerConfig};
pub use crate::shrinkers::maxxing::{MaxxingShrinker, MaxxingShrinkerConfig};
pub use crate::shrinkers::output::{InvariantShrinkerOutput, MaxxingResult, MaxxingShrinkerOutput};

mod corpus;
mod engine;
mod invariant;
mod maxxing;
mod output;
