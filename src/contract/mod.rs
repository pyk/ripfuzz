//! Solidity contract loading, artifact parsing, and call encoding.

use alloy_primitives::B256;

pub use artifact::{ContractArtifact, encode_call};
pub use builder::ContractBuilder;
pub use source_map::{
    JumpType, SourceCoverageReport, SourceHit, SourceMap, SourceMapEntry,
    resolve_coverage_to_source,
};

/// Identifier for a contract's bytecode (keccak256 hash).
pub type ContractId = B256;

pub mod artifact;
pub mod builder;
pub mod source_map;
