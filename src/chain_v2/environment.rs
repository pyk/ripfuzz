//! Environment resolution: local sandbox vs forked network.

use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use anyhow::{Context, Result};
use revm::context::{BlockEnv, CfgEnv};
use revm::database::{CacheDB, InMemoryDB};

use crate::chain_v2::Database;
use crate::chain_v2::database::ForkDb;
use crate::rpc_v2::Client;

/// Fuzzing environment.
#[derive(Debug, Clone)]
pub enum Environment {
    /// Empty sandbox. No RPC. No remote state.
    Local,
    /// Fork from a live network at a specific block.
    Fork {
        rpc: Arc<Client>,
        block_number: u64,
        block_header: BlockHeader,
        chain_id: u64,
    },
}

impl Environment {
    /// Local sandbox environment.
    pub fn local() -> Self {
        Self::Local
    }

    /// Fork environment pinned to a remote block.
    ///
    /// Fetches the block header from the RPC to initialise the block environment.
    pub fn fork(rpc: Arc<Client>, block_number: u64) -> Result<Self> {
        let block = rpc
            .get_block_by_number(block_number)
            .context("fetching block header for fork environment")?;
        let chain_id = rpc.chain_id();
        Ok(Self::Fork {
            rpc,
            block_number,
            block_header: BlockHeader {
                number: block.number.to(),
                timestamp: block.timestamp.to(),
                coinbase: block.coinbase,
                gas_limit: block.gas_limit.to(),
                basefee: block.basefee.to(),
                prevrandao: block.prevrandao,
                difficulty: block.difficulty,
                excess_blob_gas: block.excess_blob_gas.map(|v| v.to()),
            },
            chain_id,
        })
    }

    /// Consume the environment and produce the database + block + cfg configuration.
    pub fn into_components(self) -> Result<(Database, BlockEnv, CfgEnv)> {
        match self {
            Self::Local => {
                let db = Database::Sandbox(InMemoryDB::default());
                let mut block_env = BlockEnv {
                    number: U256::from(1),
                    beneficiary: Address::ZERO,
                    timestamp: U256::from(1),
                    gas_limit: u64::MAX,
                    basefee: 0,
                    difficulty: U256::ZERO,
                    prevrandao: Some(B256::ZERO),
                    blob_excess_gas_and_price: None,
                    slot_num: 0,
                };
                block_env.set_blob_excess_gas_and_price(0, 3338477);
                let mut cfg = CfgEnv::default();
                cfg.chain_id = 1;
                cfg.tx_gas_limit_cap = Some(u64::MAX);
                cfg.disable_nonce_check = true;
                Ok((db, block_env, cfg))
            }
            Self::Fork {
                rpc,
                block_number,
                block_header,
                chain_id,
            } => {
                let fork_db = ForkDb::new(rpc, block_number);
                let db = Database::Fork(CacheDB::new(fork_db));
                let mut cfg = CfgEnv::default();
                cfg.chain_id = chain_id;
                cfg.tx_gas_limit_cap = Some(u64::MAX);
                cfg.disable_nonce_check = true;
                Ok((db, block_header.into_block_env(), cfg))
            }
        }
    }
}

/// Parsed block header fields from a remote RPC node.
#[derive(Debug, Clone)]
pub struct BlockHeader {
    pub number: u64,
    pub timestamp: u64,
    pub coinbase: Address,
    pub basefee: u64,
    pub gas_limit: u64,
    pub prevrandao: Option<B256>,
    pub difficulty: U256,
    pub excess_blob_gas: Option<u64>,
}

impl BlockHeader {
    /// Convert into a revm [`BlockEnv`].
    pub fn into_block_env(self) -> BlockEnv {
        let mut env = BlockEnv {
            number: U256::from(self.number),
            beneficiary: self.coinbase,
            timestamp: U256::from(self.timestamp),
            gas_limit: self.gas_limit,
            basefee: self.basefee,
            difficulty: self.difficulty,
            prevrandao: self.prevrandao,
            blob_excess_gas_and_price: None,
            slot_num: 0,
        };
        // Cancun requires excess_blob_gas to be set.
        if let Some(excess) = self.excess_blob_gas {
            env.set_blob_excess_gas_and_price(excess, 3338477);
        }
        env
    }
}
