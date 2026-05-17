//! Corpus types for raptor: call definitions and corpus items.

pub use call::Call;
pub use corpus::{
    ContractCoverage, ContractId, Corpus, CorpusItem, CoverageMap, LocalContractCoverage,
    LocalCoverage,
};

pub mod call;
#[allow(clippy::module_inception)]
pub mod corpus;
