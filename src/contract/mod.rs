//! Solidity contract loading, artifact parsing, and call encoding.

pub use artifact::{ContractArtifact, encode_call};
pub use builder::ContractBuilder;
pub use source_map::{
    JumpType, SourceCoverageReport, SourceHit, SourceMap, SourceMapEntry,
    resolve_coverage_to_source,
};
pub mod artifact;
pub mod builder;
pub mod source_map;
