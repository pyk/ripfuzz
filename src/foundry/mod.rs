//! Foundry integration for compiling Solidity projects and loading build artifacts.
//!
//! This module has two responsibilities:
//!
//! 1. **Building a project**: compile a Foundry project via `forge build`
//!    through [`Project::build`].
//! 2. **Loading build artifacts**: read compiled artifacts from the project's
//!    `out/` directory via [`Project::load_artifacts`].

pub use artifact::{
    AbstractArtifact, Artifact, ArtifactBytecode, ArtifactId, ContractArtifact, InterfaceArtifact,
    LibraryArtifact,
};
pub use build_options::BuildOptions;
pub use project::Project;
pub mod artifact;
pub mod build_options;
pub mod project;
