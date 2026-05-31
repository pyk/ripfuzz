//! Raptor - Parallelized, coverage-guided, mutational Solidity smart contract fuzzer.

pub use corpus::{Call, CorpusConfig, ExtractedLiterals, Item, SharedCorpus};
pub use evm::{Chain, ChainConfig, Contract, DeployInput, ForkConfig, SharedCoverage, Transaction};
pub use foundry::{Artifact, ArtifactId, BuildOptions, Project};
pub use fuzzer::{Config as FuzzerConfig, FailedAssertion, Fuzzer};
pub use shrinker::{Config as ShrinkerConfig, Shrinker};

pub mod commands;
pub mod logger;

mod corpus;
mod evm;
mod foundry;
mod fuzzer;
mod reporter;
mod shrinker;
