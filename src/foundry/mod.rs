//! Foundry integration utilities for reading project artifacts, profiles, and configuration.

pub use build_artifact::{
    AbstractArtifact, BuildArtifact, BuildArtifactBytecode, BuildArtifactId, ContractArtifact,
    InterfaceArtifact, LibraryArtifact,
};
pub use project::Project;
pub mod build_artifact;
pub mod project;
