//! Raptor - Parallelized, coverage-guided, mutational Solidity smart contract fuzzer.

pub use corpus::{Call, CorpusConfig, CorpusStats, ExtractedLiterals, Item, SharedCorpus, Stats};
pub use evm::{
    Chain, ChainConfig, Contract, DeployInput, DeployOutput, ExecOutput, ForkConfig, SetupOutput,
    SharedCoverage, Trace, Transaction, TransactionResult,
};
pub use foundry::{Artifact, ArtifactId, BuildOptions, Project};
pub use fuzzer::{Config as FuzzerConfig, FailedAssertion, Fuzzer, RunOutput};
pub use shrinker::{Config as ShrinkerConfig, Shrinker, ShrinkerOutput};

pub mod commands;
pub mod logger;

mod corpus;
mod evm;
mod foundry;
mod fuzzer;
mod reporter;
mod shrinker;
