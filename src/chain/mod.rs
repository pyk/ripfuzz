//! Chain abstraction: composable, functional EVM integration.
//!
//! Provides a single entry point for deployment, setup, and sequence execution.

pub use core::{Chain, ChainBuilder, ChainConfig};
pub use executor::ExecutionOptions;
pub use fork::{CacheStats, ForkConfig, ForkDatabase};

pub mod core;
pub mod error;
pub mod executor;
pub mod fork;
pub mod init;
pub mod inspectors;
pub mod output;
pub mod setup;
pub mod state;
