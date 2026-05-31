//! Foundry integration for compiling Solidity projects and loading build artifacts.
//!
//! This module has two responsibilities:
//!
//! 1. **Building a project**: compile a Foundry project via `forge build`
//!    through [`Project::build`].
//! 2. **Loading build artifacts**: read compiled artifacts from the project's
//!    `out/` directory via [`Project::load_artifacts`].

pub use artifact::{
    Artifact, ArtifactId, ContractArtifact, LinkReferences, StorageTypeInfo,
    get_contract_definition,
};
pub use build_options::BuildOptions;
pub use project::Project;

mod artifact;
mod build_options;
mod project;
