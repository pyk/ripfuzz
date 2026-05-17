//! Raptor - Parallelized, coverage-guided, mutational Solidity smart contract fuzzer.

pub mod campaign;
pub mod commands;
pub mod contract;
pub mod corpus;
pub mod evm;
pub mod foundry;
pub mod inspector;
pub mod logger;
pub mod trace;
pub mod worker;

/// Convert a [`Result`] into an [`Option`] without the `ok()` method call.
pub(crate) fn result_to_option<T, E>(result: Result<T, E>) -> Option<T> {
    result.map_or(None, |v| Some(v))
}
