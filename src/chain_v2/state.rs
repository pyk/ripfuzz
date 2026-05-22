//! Cloneable state snapshot owned by the chain and cloned by the fuzzer.

use alloy_primitives::Address;
use revm::bytecode::Bytecode;
use revm::context::{BlockEnv, CfgEnv};
use revm::primitives::{Bytes, KECCAK_EMPTY, U256};
use revm::state::AccountInfo;

use crate::chain_v2::Database;

/// Snapshot of EVM state at a specific point in time.
///
/// The fuzzer clones this after setup to create an isolated execution context
/// for each sequence.
#[derive(Clone, Debug)]
pub struct StateSnapshot {
    pub db: Database,
    pub block_env: BlockEnv,
    pub cfg_env: CfgEnv,
    pub deployer: Address,
    pub deployer_nonce: u64,
}

impl StateSnapshot {
    /// Seed the deployer account with max balance.
    pub fn seed_deployer(&mut self, deployer: Address) {
        let info = AccountInfo {
            balance: U256::from_str_radix("ffffffffffffffffffffffffffffffff", 16)
                .unwrap_or(U256::MAX),
            nonce: 0,
            code_hash: KECCAK_EMPTY,
            code: None,
            account_id: None,
        };
        self.db.insert_account_info(deployer, info);
        self.deployer = deployer;
        self.deployer_nonce = 0;
    }

    /// Pre-deploy bytecode at an address.
    pub fn predeploy(&mut self, address: Address, code: Bytes) {
        let bytecode = Bytecode::new_raw(code);
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 0,
            code_hash: bytecode.hash_slow(),
            code: Some(bytecode),
            account_id: None,
        };
        self.db.insert_account_info(address, info);
    }
}
