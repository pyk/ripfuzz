//! Foundry integration utilities for reading project artifacts, profiles, and configuration.

pub mod artifact;
pub mod forge;
pub mod toml;

pub use artifact::ArtifactJson;
pub use toml::{FoundryProfile, FoundryToml};
