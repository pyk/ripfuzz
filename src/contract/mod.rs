//! Solidity contract loading, artifact parsing, and call encoding.

pub mod artifact;
pub mod builder;

pub use artifact::{ContractArtifact, encode_call};
pub use builder::ContractBuilder;
