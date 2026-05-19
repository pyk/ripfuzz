//! Chain state management: encapsulates everything cloned per execution.

use std::collections::HashMap;

use alloy_json_abi::JsonAbi;
use revm::{
    Database,
    primitives::{Address, Bytes, U256},
    state::AccountInfo,
};

use crate::vm::VmState;

/// The database type used for all campaigns (forked and local).
pub type ChainDatabase = crate::chain::fork::ForkDatabase;

/// Everything that must be cloned for each sequence execution.
#[derive(Clone, Debug)]
pub struct ChainState {
    pub db: ChainDatabase,
    pub block_number: u64,
    pub block_timestamp: u64,
    pub caller_nonce: u64,
    pub known_contracts: HashMap<Address, (String, JsonAbi)>,
    /// Foundry VM state owned by this chain snapshot.  Cloned per
    /// sequence so cheatcode mutations are isolated.
    pub vm: VmState,
}

impl ChainState {
    pub fn new(db: ChainDatabase) -> Self {
        Self {
            db,
            block_number: 1,
            block_timestamp: 1,
            caller_nonce: 0,
            known_contracts: HashMap::new(),
            vm: VmState::default(),
        }
    }

    /// Flush the fork cache to disk if a fork backend is present.
    pub fn flush_fork_cache(&self) -> anyhow::Result<()> {
        self.db.db.flush_cache()
    }

    /// Advance block context by the given delays.
    ///
    /// Medusa-style rules: each block gets a unique timestamp, and the first
    /// call in a sequence may stay at the same block when delays are zero.
    pub fn advance_block(&mut self, number_delay: u64, time_delay: u64, idx: usize) {
        if idx > 0 {
            // Ensure each subsequent call gets a distinct block context.
            self.block_number += number_delay.max(1);
            self.block_timestamp += time_delay.max(1);
        } else {
            self.block_number += number_delay;
            self.block_timestamp += time_delay;
        }
    }

    /// Increment and return the caller nonce.
    pub fn next_nonce(&mut self) -> u64 {
        let n = self.caller_nonce;
        self.caller_nonce += 1;
        n
    }

    // --- ChainState helpers for cheatcodes ---

    /// Set the balance of an address.
    pub fn set_balance(&mut self, addr: Address, value: U256) {
        let mut info = self.db.basic(addr).unwrap_or_default().unwrap_or_default();
        info.balance = value;
        self.db.insert_account_info(addr, info);
    }

    /// Set the code of an address.
    pub fn set_code(&mut self, addr: Address, code: Bytes) {
        let mut info = self.db.basic(addr).unwrap_or_default().unwrap_or_default();
        let bytecode = revm::bytecode::Bytecode::new_raw(code);
        info.code_hash = bytecode.hash_slow();
        info.code = Some(bytecode);
        self.db.insert_account_info(addr, info);
    }

    /// Set a storage slot for an address.
    pub fn set_storage(&mut self, addr: Address, slot: U256, value: U256) {
        // InMemoryDB exposes `insert_account_storage` via the Database trait.
        let _ = self.db.insert_account_storage(addr, slot, value);
    }

    /// Read a storage slot for an address.
    pub fn load_storage(&mut self, addr: Address, slot: U256) -> U256 {
        self.db.storage(addr, slot).unwrap_or_default()
    }

    /// Set the nonce of an address.
    pub fn set_nonce(&mut self, addr: Address, nonce: u64) {
        let mut info = self.db.basic(addr).unwrap_or_default().unwrap_or_default();
        info.nonce = nonce;
        self.db.insert_account_info(addr, info);
    }

    /// Get the nonce of an address.
    pub fn get_nonce(&mut self, addr: Address) -> u64 {
        self.db
            .basic(addr)
            .unwrap_or_default()
            .unwrap_or_default()
            .nonce
    }

    /// Get or create an account's info.
    pub fn account_info(&mut self, addr: Address) -> AccountInfo {
        self.db.basic(addr).unwrap_or_default().unwrap_or_default()
    }
}
