//! Persistent cheatcode state types.

use std::collections::HashMap;
use std::path::PathBuf;

use revm::primitives::{Address, Bytes, U256};

/// Record of a balance change produced by `vm.deal`.
#[derive(Clone, Debug)]
pub struct EthDealRecord {
    pub address: Address,
    pub old_balance: U256,
}

/// Record of a nonce change produced by `vm.setNonce`.
#[derive(Clone, Debug)]
pub struct NonceChangeRecord {
    pub address: Address,
    pub old_nonce: u64,
}

/// Transient scratchpad for one call sequence.
#[derive(Clone, Debug, Default)]
pub struct ExecutionState {
    pub block: BlockCheatState,
    pub prank: PrankCheatState,
    pub labels: HashMap<Address, String>,
    pub compiled_contracts: HashMap<String, Bytes>,
    pub project_root: PathBuf,
    pub ffi_enabled: bool,
    pub eth_deals: Vec<EthDealRecord>,
    pub nonce_changes: Vec<NonceChangeRecord>,
}

impl ExecutionState {
    /// Seed execution state from a [`Config`](crate::evm::cheatcode::Config).
    pub fn from_config(config: &crate::evm::cheatcode::Config) -> Self {
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

impl PrankCheatState {
    pub fn caller_for_top_level(&self) -> Option<Address> {
        self.start.as_ref().map(|s| s.caller)
    }

    pub fn origin_for_top_level(&self, default: Address) -> Address {
        self.start
            .as_ref()
            .and_then(|s| s.origin)
            .unwrap_or(default)
    }
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
