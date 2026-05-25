//! Foundry-compatible cheatcode inspector.
//!
//! The main entrypoint is [`Inspector`](inspector::Inspector), configured
//! via [`Config`].

pub use address::VM_ADDRESS;
pub use config::Config;
pub use inspector::Inspector;
pub use state::{BlockCheatState, ExecutionState, PrankCheatState, PrankState, StartPrankState};

pub mod address;
pub mod config;
pub mod inspector;
pub mod outcome;
pub mod state;

pub mod calls;
