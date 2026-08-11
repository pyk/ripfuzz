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
pub use fuzzer::{
    FailedAssertion, FunctionMetricsSnapshot, Fuzzer, FuzzerConfig, FuzzerOutput,
    SharedFailedAssertions, SharedMetrics, Snapshot,
};
pub use max::{
    MaxBestItem, MaxFuzzer, MaxFuzzerConfig, MaxFuzzerCorpus, MaxFuzzerOutput, MaxObjective,
    MaxResult, MaxShrinker, MaxShrinkerConfig, MaxShrinkerCorpus, MaxShrinkerOutput,
};
pub use shrinker::{Shrinker, ShrinkerConfig, ShrinkerOutput};

pub mod commands;
pub mod logger;

mod campaigns;
mod corpus;
mod evm;
mod formatter;
mod foundry;
mod fuzzer;
mod max;
mod shrinker;
