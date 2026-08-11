//! Ripfuzz - High-throughput, coverage-guided, mutational fuzzer for Solidity smart contracts.

pub use corpus::{
    Call, CorpusConfig, CorpusReplayer, CorpusStats, ExtractedLiterals, Item, SharedCorpus,
    SharedFailedCorpusItem, Stats,
};
pub use evm::{
    CallFrame, CallFrameKind, Chain, ChainConfig, CheatcodeConfig, Contract, CoverageReport,
    CoverageReporter, CoverageUpdate, DEFAULT_DEPLOYER, DeployInput, DeployLibraryInput,
    DeployLibraryOutput, DeployOutput, ExecOutput, ExecutionContractCoverage, ExecutionCoverage,
    ForkDBConfig, MockTransport, SetupInput, SetupOutput, SharedCoverage, StorageChange,
    StorageChangeInfo, StorageType, Trace, TraceContext, TraceDisplay, Transaction,
    TransactionResult,
};
pub use foundry::{Artifact, ArtifactId, BuildOptions, Project};
pub use fuzzers::{
    FailedAssertion, FunctionMetricsSnapshot, InvariantFuzzer, InvariantFuzzerConfig,
    InvariantFuzzerOutput, MaxBestItem, MaxObjective, MaxxingFuzzer, MaxxingFuzzerConfig,
    MaxxingFuzzerCorpus, MaxxingFuzzerOutput, SharedFailedAssertions, SharedMetrics,
    SharedStopEvent, Snapshot, StopEvent,
};
pub use shrinkers::{
    InvariantShrinker, InvariantShrinkerConfig, InvariantShrinkerOutput, MaxxingResult,
    MaxxingShrinker, MaxxingShrinkerConfig, MaxxingShrinkerCorpus, MaxxingShrinkerOutput,
};

pub mod commands;
pub mod logger;

mod campaigns;
mod corpus;
mod evm;
mod formatter;
mod foundry;
mod fuzzers;
mod shrinkers;
