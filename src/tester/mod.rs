//! Find findings.
//!
//! The `tester` module mirrors [`crate::max`] around a different objective:
//! instead of maximizing a value, the fuzzers hunt explicit `rvm.finding`
//! reports, both inside handler calls and inside `invariant_*` functions
//! checked after each handler call.
//!
//! ```rust,no_run
//! use ripfuzz::tester::{Corpus, Fuzzer, Shrinker, TestHarness};
//! use ripfuzz::{Chain, ChainConfig, SharedCoverage};
//!
//! # let solc_output: ripfuzz::compilers::solc::SolcOutput = todo!();
//! # let chain = Chain::empty(ChainConfig::default());
//! # let coverage = SharedCoverage::new();
//! // 1. Validate the compiled harness.
//! // let test_harness = TestHarness::try_from(&solc_output)?;
//! // 2. Deploy it and fuzz for findings.
//! // let deployment = chain.deploy(&test_harness)?;
//! // let output = Fuzzer::new()
//! //     .with_chain(chain)
//! //     .with_corpus(Corpus::new())
//! //     .with_coverage(coverage)
//! //     .with_findings(ripfuzz::tester::SharedFindings::new(256))
//! //     .run()?;
//! // 3. Shrink every finding's sequence.
//! // let findings = Shrinker::new().shrink(&output.findings)?;
//! ```

pub use crate::evm::Severity;
pub use corpus::{Call, Corpus, EntrySnapshot, LiteralExtractor, Replayer, Sequence};
pub use finding::{Finding, SharedFindings};
pub use fuzzer::{Fuzzer, Output};
pub use harness::TestHarness;
pub use shrinker::Shrinker;

mod corpus;
mod finding;
mod fuzzer;
mod harness;
mod shrinker;
