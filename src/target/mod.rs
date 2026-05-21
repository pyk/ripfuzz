//! Target contract definition and validation.
//!
//! This module transforms a Foundry build artifact into a validated target
//! contract ready for fuzzing. It has three responsibilities:
//!
//! 1. **Contract deployment**: extract initcode from the artifact so the fuzzer
//!    can deploy the contract via [`Contract::initcode`].
//! 2. **Contract setup**: identify an optional `setup` function that is called
//!    once after deployment via [`Contract::setup_function`].
//! 3. **Contract fuzzing**: classify functions into target functions and
//!    invariant functions via [`Contract::target_functions`] and
//!    [`Contract::invariant_functions`].

pub use contract::Contract;
pub mod contract;
