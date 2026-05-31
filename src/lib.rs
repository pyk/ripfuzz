//! Raptor - Parallelized, coverage-guided, mutational Solidity smart contract fuzzer.

pub use corpus::{
    Call, CorpusConfig, CorpusReplayer, CorpusStats, ExtractedLiterals, Item, SharedCorpus,
    SharedFailedCorpusItem, Stats,
};
pub use evm::{
    Chain, ChainConfig, Contract, DeployInput, DeployLibraryInput, DeployOutput, ExecOutput,
    ExecutionCoverage, ForkDBConfig, SetupInput, SetupOutput, SharedCoverage, Trace, Transaction,
    TransactionResult,
};
pub use foundry::{Artifact, ArtifactId, BuildOptions, Project};
pub use fuzzer::{
    FailedAssertion, FunctionMetricsSnapshot, Fuzzer, FuzzerConfig, RunOutput, SharedMetrics,
    Snapshot,
};
pub use shrinker::{Shrinker, ShrinkerConfig, ShrinkerOutput};

pub mod commands;
pub mod logger;

mod corpus;
mod evm;
mod formatter;
mod foundry;
mod fuzzer;
mod reporter;
mod shrinker;
