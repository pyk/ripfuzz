//! Per-thread fuzzers for invariant and maxxing campaigns.
//!
//! [`InvariantFuzzer`] executes call sequences and reports failed assertions.
//! [`MaxxingFuzzer`] executes call sequences followed by the `max_*` function
//! and tracks the highest returned value.

pub use crate::fuzzers::assertions::{FailedAssertion, SharedFailedAssertions};
pub use crate::fuzzers::corpus::{MaxBestItem, MaxxingFuzzerCorpus};
pub use crate::fuzzers::invariant::{InvariantFuzzer, InvariantFuzzerConfig};
pub use crate::fuzzers::maxxing::{MaxxingFuzzer, MaxxingFuzzerConfig};
pub use crate::fuzzers::metrics::{FunctionMetricsSnapshot, SharedMetrics, Snapshot};
pub use crate::fuzzers::objective::MaxObjective;
pub use crate::fuzzers::output::{InvariantFuzzerOutput, MaxxingFuzzerOutput};
pub use crate::fuzzers::stop::{SharedStopEvent, StopEvent};

mod assertions;
mod corpus;
mod engine;
mod invariant;
mod maxxing;
mod metrics;
mod objective;
mod output;
mod stop;
