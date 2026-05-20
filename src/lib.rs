//! Raptor - Parallelized, coverage-guided, mutational Solidity smart contract fuzzer.

pub mod campaign;
pub mod chain;
pub mod commands;
pub mod contract;
pub mod corpus;
pub mod coverage;
pub mod foundry;
pub mod fuzzer;
pub mod logger;
pub mod rpc;
pub mod target;
pub mod vm;

/// Convert a [`Result`] into an [`Option`] without the `ok()` method call.
pub fn result_to_option<T, E>(result: Result<T, E>) -> Option<T> {
    result.map_or(None, |v| Some(v))
}
