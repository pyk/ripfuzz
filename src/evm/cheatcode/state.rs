//! Persistent cheatcode state types used by [`BaseState`](crate::chain::BaseState)
//! and [`ExecutionState`](crate::evm::cheatcode::ExecutionState).

use alloy_primitives::U256;
use revm::primitives::Address;

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
    /// Stored in prank state so it survives EVM rebuilds.
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

/// Block-context overrides produced from [`BlockCheatState`].
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
