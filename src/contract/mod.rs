//! Solidity contract loading, artifact parsing, and call encoding.

pub use artifact::{ContractArtifact, encode_call};
pub use builder::ContractBuilder;
pub mod artifact;
pub mod builder;
