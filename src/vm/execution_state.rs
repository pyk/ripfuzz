//! Transient scratchpad for one call sequence.

use std::collections::HashMap;
use std::path::PathBuf;

use revm::primitives::{Address, Bytes};

use crate::vm::{BlockCheatState, DealRecord, NonceRecord, PrankCheatState};

/// Transient scratchpad for one call sequence.
/// Lives inside [`CheatcodeInspector`](crate::vm::inspector::CheatcodeInspector).
/// Born per sequence, dropped per sequence.
#[derive(Clone, Debug, Default)]
pub struct ExecutionState {
    // Seeded from BaseState at sequence start:
    pub project_root: PathBuf,
    pub ffi_enabled: bool,
    pub compiled_contracts: HashMap<String, Bytes>,
    pub labels: HashMap<Address, String>,
    pub prank: PrankCheatState,
    pub block: BlockCheatState,

    // Fresh per sequence:
    pub eth_deals: Vec<DealRecord>,
    pub nonce_changes: Vec<NonceRecord>,
}

impl ExecutionState {
    /// Return all block-context overrides that should be applied before a call.
    pub fn block_overrides(&self) -> crate::vm::BlockOverrides {
        crate::vm::BlockOverrides {
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
