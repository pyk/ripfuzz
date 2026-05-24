//! Foundry-compatible VM contract: cheatcodes, state, and dispatch.
//!
//! The public surface is intentionally small: [`VM_ADDRESS`] and [`VmConfig`].
//! Everything else is `pub` so that `chain/` can access it, but the module
//! structure makes the boundary clear.

pub use address::VM_ADDRESS;
pub use config::VmConfig;
pub use decode::{
    decode_address_arg, decode_address_bytes32_args, decode_address_bytes32_bytes32_args,
    decode_address_u256_args, decode_u256_arg,
};
pub use dispatch::{Cheatcode, dispatch_effects};
pub use effect::CheatcodeEffect;
pub use execution_state::ExecutionState;
pub use functions::deal::DealRecord;
pub use functions::nonce::NonceRecord;
pub use outcome::{
    build_outcome, dummy_success, panic_outcome, revert_outcome, success_bool_outcome,
    success_bytes_outcome, success_int256_outcome, success_u256_outcome,
};
pub use state::{BlockCheatState, BlockOverrides, PrankCheatState, PrankState, StartPrankState};
pub use vm::Vm;

/// Trait for VM factories that expose configuration.
pub trait VmFactory: Send + Sync + std::fmt::Debug + 'static {
    /// Access the VM configuration.
    fn config(&self) -> &VmConfig;
}

pub mod address;
pub mod config;
pub mod decode;
pub mod dispatch;
pub mod effect;
pub mod execution_state;
pub mod functions;
pub mod inspector;
pub mod outcome;
pub mod state;
#[allow(clippy::module_inception)]
pub mod vm;

pub mod test_harness;
