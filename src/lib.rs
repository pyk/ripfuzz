//! Raptor - Parallelized, coverage-guided, mutational Solidity smart contract fuzzer.

pub use corpus::{
    Call, CorpusConfig, CorpusReplayer, CorpusStats, ExtractedLiterals, Item, SharedCorpus,
    SharedFailedCorpusItem, Stats,
};
pub use evm::{
    CallFrame, CallFrameKind, Chain, ChainConfig, CheatcodeConfig, Contract, CoverageReport,
    CoverageReporter, CoverageUpdate, DeployInput, DeployLibraryInput, DeployLibraryOutput,
    DeployOutput, ExecOutput, ExecutionContractCoverage, ExecutionCoverage, ForkDBConfig,
    SetupInput, SetupOutput, SharedCoverage, StorageChange, StorageChangeInfo, StorageType, Trace,
    TraceContext, TraceDisplay, Transaction, TransactionResult,
};
pub use foundry::{Artifact, ArtifactId, BuildOptions, Project};
pub use fuzzer::{
    FailedAssertion, FunctionMetricsSnapshot, Fuzzer, FuzzerConfig, FuzzerOutput, SharedMetrics,
    Snapshot,
};
pub use shrinker::{Shrinker, ShrinkerConfig, ShrinkerOutput};

pub mod commands;
pub mod logger;

mod console;
mod corpus;
mod evm;
mod formatter;
mod foundry;
mod fuzzer;
mod shrinker;
