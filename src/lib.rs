//! Ripfuzz - An extremely fast Smart contract fuzzer.

pub use evm::{
    CallFrame, CallFrameKind, Chain, ChainConfig, CheatcodeConfig, CoverageId, CoverageReport,
    CoverageReporter, CoverageUpdate, CoverageWriter, DEFAULT_DEPLOYER, DeployInput,
    DeployLibraryInput, DeployLibraryOutput, DeployOutput, Evmole, ExecOutput,
    ExecutionContractCoverage, ExecutionCoverage, ExecutionTraceWriter, ForkDBConfig,
    MockTransport, RpcStats, SetupInput, SetupOutput, SharedCoverage, StorageChange,
    StorageChangeInfo, StorageType, Trace, TraceContext, TraceDisplay, Transaction,
    TransactionResult,
};
pub use tester::{
    BrokenInvariant, BrokenInvariantReporter, Corpus as TestCorpus, Fuzzer as TestFuzzer,
    Shrinker as TestShrinker, TestHarness,
};

pub mod cli;
pub mod compilers;
pub mod config;
pub mod dependencies;
pub mod executor;
pub mod harness;
pub mod inspectors;
pub mod logger;
pub mod maxer;
pub mod tester;

mod evm;
