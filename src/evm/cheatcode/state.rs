//! Persistent cheatcode state types.

use std::collections::HashMap;
use std::path::PathBuf;

use revm::primitives::{Address, Bytes, U256};

use crate::evm::cheatcode::CheatcodeConfig;

/// Transient scratchpad for one call sequence.
#[derive(Clone, Debug, Default)]
pub struct ExecutionState {
    pub block: BlockCheatState,
    pub prank: PrankCheatState,
    pub labels: HashMap<Address, String>,
    pub compiled_contracts: HashMap<String, Bytes>,
    pub project_root: PathBuf,
    pub ffi_enabled: bool,
}

impl ExecutionState {
    // TODO(pyk): remove this, Chain owns execution state now
    /// Seed execution state from a [`CheatcodeConfig`].
    pub fn from_config(config: &CheatcodeConfig) -> Self {
        Self {
            project_root: config.project_root.clone(),
            ffi_enabled: config.ffi,
            compiled_contracts: config.compiled_contracts.clone(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BlockCheatState {
    pub timestamp: Option<U256>,
    pub number: Option<U256>,
    pub basefee: Option<U256>,
    pub beneficiary: Option<Address>,
    pub prevrandao: Option<revm::primitives::FixedBytes<32>>,
    pub chain_id: Option<U256>,
}

#[derive(Clone, Debug, Default)]
pub struct PrankCheatState {
    pub active: Option<PrankState>,
    pub start: Option<StartPrankState>,
    pub original_origin: Option<Address>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrankState {
    pub caller: Address,
    pub origin: Option<Address>,
    pub single_call: bool,
    pub set_depth: u64,
    pub prank_caller: Address,
    pub used: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartPrankState {
    pub caller: Address,
    pub origin: Option<Address>,
    pub set_depth: u64,
    pub prank_caller: Address,
    pub used: bool,
}
