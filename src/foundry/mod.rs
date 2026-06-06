//! Foundry integration for compiling Solidity projects and loading build artifacts.
//!
//! This module has three responsibilities:
//!
//! 1. **Building a project**: compile a Foundry project via `forge build`
//!    through [`Project::build`].
//! 2. **Loading build artifacts**: read compiled artifacts from the project's
//!    `out/` directory via [`Project::load_artifacts`].
//! 3. **Loading build info**: read compiler build-info files from the project's
//!    `out/build-info/` directory via [`BuildInfo::load_source_index_for_artifact`].

pub use artifact::{
    Artifact, ArtifactBytecode, ArtifactId, ContractArtifact, LinkReferences, StorageTypeInfo,
};
pub use build_info::BuildInfo;
pub use build_options::BuildOptions;
pub use project::Project;

mod artifact;
pub mod build_info;
mod build_options;
mod project;
