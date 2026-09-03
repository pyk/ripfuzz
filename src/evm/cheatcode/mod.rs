//! Foundry-compatible cheatcode inspector.
//!
//! The main entrypoint is [`Inspector`](inspector::Inspector), configured
//! via [`Config`].

pub use address::VM_ADDRESS;
pub use config::CheatcodeConfig;
pub use inspector::Inspector;
pub use state::ExecutionState;

mod address;
mod config;
mod inspector;
mod outcome;
mod state;

mod calls;
