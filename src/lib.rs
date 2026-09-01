//! Ripfuzz - An extremely fast Smart contract fuzzer.

pub use corpus::{
    Call, CorpusConfig, CorpusReplayer, CorpusStats, ExtractedLiterals, Item, ReplayFailure,
    SharedCorpus, SharedFailedCorpusItem, Stats,
};
pub use evm::{
    CallFrame, CallFrameKind, Chain, ChainConfig, CheatcodeConfig, Contract, CoverageId,
    CoverageReport, CoverageReporter, CoverageUpdate, DEFAULT_DEPLOYER, DeployInput,
    DeployLibraryInput, DeployLibraryOutput, DeployOutput, Evmole, ExecOutput,
    ExecutionContractCoverage, ExecutionCoverage, ForkDBConfig, MockTransport, RpcStats,
    SetupInput, SetupOutput, SharedCoverage, StorageChange, StorageChangeInfo, StorageType, Trace,
    TraceContext, TraceDisplay, Transaction, TransactionResult,
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
pub use tester::{
    BrokenInvariant, Corpus as TestCorpus, Fuzzer as TestFuzzer, Shrinker as TestShrinker,
    TestHarness,
};

pub mod cli;
pub mod compilers;
pub mod config;
pub mod exec;
pub mod harness;
pub mod logger;
pub mod max;
pub mod tester;

mod campaigns;
mod corpus;
mod evm;
mod formatter;
mod foundry;
mod fuzzers;
mod shrinkers;
