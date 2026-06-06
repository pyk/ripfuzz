//! revm-native EVM chain abstraction.
//!
//! `evm::Chain` owns EVM state and executes raw transactions. It does not know
//! about fuzzer concepts (invariants, corpus calls, labels, or contract setup).
//! Those concerns live in the caller.
//!
//! Responsibilities:
//! 1. Own EVM state: [`BlockEnv`](revm::context::BlockEnv),
//!    [`CfgEnv`](revm::context::CfgEnv), and a [`Database`].
//! 2. Execute raw transactions: [`Chain::deploy`], [`Chain::call`],
//!    [`Chain::transact`].
//! 3. Return pure data structures for traces; never format or print them.

pub use chain::ChainConfig;
pub use chain::{
    Chain, DEFAULT_DEPLOYER, DeployInput, DeployLibraryInput, DeployLibraryOutput, DeployOutput,
    ExecOutput, SetupInput, SetupOutput, Transaction,
};
pub use cheatcode::CheatcodeConfig;
pub use contract::Contract;
pub use coverage::{
    CoverageReport, CoverageReporter, CoverageUpdate, ExecutionContractCoverage, ExecutionCoverage,
    SharedCoverage,
};
pub use forkdb::ForkDBConfig;
pub use result::TransactionResult;
pub use trace::{
    CallFrame, CallFrameKind, StorageChange, StorageChangeInfo, StorageType, Trace, TraceContext,
    TraceDisplay,
};

mod chain;
mod cheatcode;
mod contract;
mod coverage;
mod database;
mod forkdb;
mod result;
mod specs;
mod trace;
