//! Persistent cheatcode state types.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use revm::primitives::{Address, U256};

use crate::evm::cheatcode::CheatcodeConfig;
use crate::evm::forkdb::{ForkDBConfig, SharedLocalAddressRegistry, Transport};

/// Transient scratchpad for one call sequence.
#[derive(Clone, Debug)]
pub struct ExecutionState {
    pub block: BlockCheatState,
    pub prank: PrankCheatState,
    pub labels: HashMap<Address, String>,
    pub project_root: PathBuf,
    pub ffi_enabled: bool,
    /// Default RPC settings used by `vm.fork` when no per-call config is given.
    pub fork_defaults: ForkDBConfig,
    /// Optional transport override (tests inject [`crate::evm::MockTransport`]).
    pub transport: Option<Arc<dyn Transport>>,
    /// Local addresses that must persist across fork switches.
    pub local_registry: SharedLocalAddressRegistry,
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self {
            block: BlockCheatState::default(),
            prank: PrankCheatState::default(),
            labels: HashMap::new(),
            project_root: PathBuf::new(),
            ffi_enabled: false,
            fork_defaults: ForkDBConfig::new(""),
            transport: None,
            local_registry: SharedLocalAddressRegistry::new(),
        }
    }
}

impl ExecutionState {
    /// Seed execution state from a [`CheatcodeConfig`].
    pub fn from_config(config: &CheatcodeConfig) -> Self {
        Self {
            project_root: config.project_root.clone(),
            ffi_enabled: config.ffi,
            fork_defaults: config.fork_defaults.clone(),
            transport: config.transport.clone(),
            local_registry: SharedLocalAddressRegistry::new(),
            ..Self::default()
        }
    }

    /// Attach the chain's shared local-address registry.
    pub fn with_local_registry(mut self, registry: SharedLocalAddressRegistry) -> Self {
        self.local_registry = registry;
        self
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
