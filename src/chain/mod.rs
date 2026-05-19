//! Chain abstraction: composable, functional EVM integration.
//!
//! Provides a single entry point for deployment, setup, and sequence execution.

pub use base_state::BaseState;
pub use core::{Chain, ChainBuilder, ChainConfig};
pub use database::{CacheStats, Database};
pub use environment::Environment;
pub use executor::{ExecutionOptions, SequenceExecutor};
pub use forkdb::{ForkDB, ForkDBBuilder};

pub mod base_state;
pub mod core;
pub mod database;
pub mod environment;
pub mod error;
pub mod executor;
pub mod forkdb;
pub mod init;
pub mod inspectors;
pub mod output;
pub mod setup;
