//! Find broken invariants.
//!
//! The `tester` module mirrors [`crate::maxer`] around a different objective:
//! instead of maximizing a value, the fuzzers hunt explicit `BrokenInvariantError`
//! reports, both inside handler calls and inside `invariant_*` functions
//! checked after each handler call.
//!
//! ```rust,no_run
//! use ripfuzz::tester::{BrokenInvariant, Corpus, Fuzzer, Shrinker, SharedBrokenInvariants, TestHarness};
//! use ripfuzz::evm::{Chain, ChainConfig, SharedCoverage};
//!
//! # let solc_output: ripfuzz::compilers::solc::SolcOutput = todo!();
//! # let chain = Chain::empty(ChainConfig::default());
//! # let coverage = SharedCoverage::new();
//! // 1. Validate the compiled harness.
//! // let test_harness = TestHarness::try_from(&solc_output)?;
//! // 2. Deploy it and fuzz for broken invariants.
//! // let deployment = chain.deploy(&test_harness)?;
//! // let output = Fuzzer::new()
//! //     .with_chain(chain)
//! //     .with_corpus(Corpus::new())
//! //     .with_coverage(coverage)
//! //     .with_broken_invariants(SharedBrokenInvariants::new(256))
//! //     .run()?;
//! // 3. Shrink every broken invariant's sequence.
//! // let broken_invariants = Shrinker::new().shrink(&output.broken_invariants)?;
//! ```

pub use broken_invariant::{BrokenInvariant, BrokenInvariantReporter, SharedBrokenInvariants};
pub use corpus::{Call, Corpus, EntrySnapshot, LiteralExtractor, Replayer, Sequence};
pub use fuzzer::{Fuzzer, Output};
pub use harness::TestHarness;
pub use shrinker::Shrinker;
pub use stats::{
    FunctionStats, RevertSummary, RpcSummary, SharedStats, Stats, StatsMetadata, StatsWriter,
    WallTime,
};

mod broken_invariant;
mod corpus;
mod fuzzer;
mod harness;
mod shrinker;
mod stats;
