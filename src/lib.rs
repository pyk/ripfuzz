//! Raptor - Parallelized, coverage-guided, mutational Solidity smart contract fuzzer.

pub use corpus::{Call, CorpusConfig, CorpusStats, ExtractedLiterals, Item, SharedCorpus, Stats};
pub use evm::{
    Chain, ChainConfig, Contract, DeployInput, DeployLibraryInput, DeployOutput, ExecOutput,
    ForkDBConfig, SetupInput, SetupOutput, SharedCoverage, Trace, Transaction, TransactionResult,
};
pub use foundry::{Artifact, ArtifactId, BuildOptions, Project};
pub use fuzzer::{FailedAssertion, Fuzzer, FuzzerConfig, RunOutput};
pub use shrinker::{Shrinker, ShrinkerConfig, ShrinkerOutput};

pub mod commands;
pub mod logger;

mod corpus;
mod evm;
mod foundry;
mod fuzzer;
mod reporter;
mod shrinker;
mod stats_formatter;
