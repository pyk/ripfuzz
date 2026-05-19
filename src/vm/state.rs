//! Persistent VM state accumulated by cheatcodes during execution.

use std::collections::HashMap;
use std::path::PathBuf;

use alloy_primitives::U256;
use revm::primitives::{Address, Bytes};

use crate::vm::{DealRecord, NonceRecord};

/// Persistent block-context overrides set by cheatcodes.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockCheatState {
    pub timestamp: Option<U256>,
    pub number: Option<U256>,
    pub basefee: Option<U256>,
    pub beneficiary: Option<Address>,
    pub prevrandao: Option<[u8; 32]>,
    pub chain_id: Option<U256>,
}

/// Persistent prank state set by cheatcodes.
#[derive(Clone, Debug, Default)]
pub struct PrankCheatState {
    pub active: Option<PrankState>,
    pub start: Option<StartPrankState>,
    /// The original `tx.origin` before any prank was applied.
    /// Stored in `VmState` so it survives EVM rebuilds.
    pub original_origin: Option<Address>,
}

impl PrankCheatState {
    /// Return the caller that should be used for the top-level transaction.
    pub fn caller_for_top_level(&self) -> Option<Address> {
        self.start.as_ref().map(|s| s.caller)
    }

    /// Return the origin that should be used for the top-level transaction.
    pub fn origin_for_top_level(&self, default: Address) -> Address {
        self.start
            .as_ref()
            .and_then(|s| s.origin)
            .unwrap_or(default)
    }
}

/// State accumulated by cheatcodes during execution.
#[derive(Clone, Debug, Default)]
pub struct VmState {
    pub block: BlockCheatState,
    pub prank: PrankCheatState,
    pub labels: HashMap<Address, String>,
    pub ffi_enabled: bool,
    /// Foundry project root used as the working directory for `vm.ffi`.
    pub project_root: PathBuf,
    /// Contract name -> initcode bytes, populated from the artifact so
    /// `vm.getCode` can resolve contracts by name.
    pub compiled_contracts: HashMap<String, Bytes>,
    /// Rollback records for `vm.deal` (Foundry semantics).
    pub eth_deals: Vec<DealRecord>,
    /// Rollback records for `vm.setNonce`.
    pub nonce_changes: Vec<NonceRecord>,
}

impl VmState {
    /// Return all block-context overrides that should be applied before a call.
    pub fn block_overrides(&self) -> BlockOverrides {
        BlockOverrides {
            timestamp: self.block.timestamp,
            number: self.block.number,
            basefee: self.block.basefee.map(|f| u64::try_from(f).unwrap_or(0)),
            beneficiary: self.block.beneficiary,
            prevrandao: self
                .block
                .prevrandao
                .map(revm::primitives::FixedBytes::from),
            chain_id: self
                .block
                .chain_id
                .map(|id| u64::try_from(id).unwrap_or(u64::MAX)),
        }
    }
}

/// Block-context overrides produced from `VmState`.
#[derive(Clone, Debug, Default)]
pub struct BlockOverrides {
    pub timestamp: Option<U256>,
    pub number: Option<U256>,
    pub basefee: Option<u64>,
    pub beneficiary: Option<Address>,
    pub prevrandao: Option<revm::primitives::FixedBytes<32>>,
    pub chain_id: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrankState {
    pub caller: Address,
    pub origin: Option<Address>,
    pub single_call: bool,
    /// Call depth of the frame that configured this prank.
    pub set_depth: u64,
    /// Address of the contract that called the cheatcode (prank initiator).
    pub prank_caller: Address,
    /// Whether the prank has been applied at least once.
    pub used: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartPrankState {
    pub caller: Address,
    pub origin: Option<Address>,
    /// Call depth at which this prank was set.
    pub set_depth: u64,
    /// Address of the contract that called the cheatcode (prank initiator).
    pub prank_caller: Address,
    /// Whether the prank has been applied at least once.
    pub used: bool,
}
