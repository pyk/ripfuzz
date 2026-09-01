//! Persistent cheatcode state types.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use revm::primitives::{Address, Bytes, U256};

use crate::evm::cheatcode::CheatcodeConfig;
use crate::evm::forkdb::{ForkDBConfig, SharedLocalAddressRegistry, Transport};

/// Severity of an explicit `rvm.finding` report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Severity {
    Info = 0,
    Low = 1,
    #[default]
    Medium = 2,
    High = 3,
    Critical = 4,
}

impl Severity {
    /// Convert a `uint8` cheatcode value into a [`Severity`].
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Info),
            1 => Some(Self::Low),
            2 => Some(Self::Medium),
            3 => Some(Self::High),
            4 => Some(Self::Critical),
            _ => None,
        }
    }

    /// Render the severity as a lowercase string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// One explicit finding emitted via `rvm.finding`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportedFinding {
    pub id: String,
    pub severity: Severity,
    pub title: String,
    pub description: String,
}

/// Transient scratchpad for one call sequence.
#[derive(Clone, Debug)]
pub struct ExecutionState {
    pub block: BlockCheatState,
    pub prank: PrankCheatState,
    pub labels: HashMap<Address, String>,
    pub compiled_contracts: HashMap<String, Bytes>,
    pub project_root: PathBuf,
    pub ffi_enabled: bool,
    /// Default RPC settings used by `vm.fork` when no per-call config is given.
    pub fork_defaults: ForkDBConfig,
    /// Optional transport override (tests inject [`crate::evm::MockTransport`]).
    pub transport: Option<Arc<dyn Transport>>,
    /// Local addresses that must persist across fork switches.
    pub local_registry: SharedLocalAddressRegistry,
    /// Explicit findings emitted via `rvm.finding` during the current `exec`.
    pub findings: Vec<ReportedFinding>,
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self {
            block: BlockCheatState::default(),
            prank: PrankCheatState::default(),
            labels: HashMap::new(),
            compiled_contracts: HashMap::new(),
            project_root: PathBuf::new(),
            ffi_enabled: false,
            fork_defaults: ForkDBConfig::new(""),
            transport: None,
            local_registry: SharedLocalAddressRegistry::new(),
            findings: Vec::new(),
        }
    }
}

impl ExecutionState {
    /// Seed execution state from a [`CheatcodeConfig`].
    pub fn from_config(config: &CheatcodeConfig) -> Self {
        Self {
            project_root: config.project_root.clone(),
            ffi_enabled: config.ffi,
            compiled_contracts: config.compiled_contracts.clone(),
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
