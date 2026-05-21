//! Environment resolution: local sandbox vs forked network.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use anyhow::{Context, Result};
use revm::context::{BlockEnv, CfgEnv};
use revm::database::{CacheDB, InMemoryDB};

use crate::chain_v2::Database;
use crate::chain_v2::database::ForkDb;
use crate::rpc::RpcClient;

/// Fuzzing environment.
#[derive(Debug, Clone)]
pub enum Environment {
    /// Empty sandbox. No RPC. No remote state.
    Local,
    /// Fork from a live network at a specific block.
    Fork {
        rpc: Arc<dyn RpcClient>,
        block_number: u64,
        block_header: BlockHeader,
        chain_id: u64,
        cache_dir: PathBuf,
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
    pub fn fork(
        rpc: Arc<dyn RpcClient>,
        block_number: u64,
        cache_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let header = fetch_block_header(&*rpc, block_number)
            .context("fetching block header for fork environment")?;
        let chain_id = rpc.cache_key().parse::<u64>().unwrap_or(0);
        Ok(Self::Fork {
            rpc,
            block_number,
            block_header: header,
            chain_id,
            cache_dir: cache_dir.as_ref().to_path_buf(),
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
                cache_dir,
            } => {
                let fork_db = ForkDb::new(rpc, block_number, &cache_dir)
                    .context("initialising fork database")?;
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
        // Cancun requires excess_blob_gas to be set. Use mainnet default.
        env.set_blob_excess_gas_and_price(0, 3338477);
        env
    }
}

fn fetch_block_header(rpc: &dyn RpcClient, block_number: u64) -> Result<BlockHeader> {
    let block_hex = format!("0x{:x}", block_number);
    let result = rpc
        .call(
            "eth_getBlockByNumber",
            &[
                serde_json::Value::String(block_hex),
                serde_json::Value::Bool(false),
            ],
        )
        .context("fetching fork block header")?;

    let number = parse_u64_field(&result, "number")?.unwrap_or(block_number);
    let timestamp = parse_u64_field(&result, "timestamp")?.unwrap_or(0);
    let coinbase = parse_address_field(&result, "miner")?.unwrap_or(Address::ZERO);
    let gas_limit = parse_u64_field(&result, "gasLimit")?.unwrap_or(30_000_000);
    let basefee = parse_u64_field(&result, "baseFeePerGas")?.unwrap_or(0);
    let difficulty = parse_u256_field(&result, "difficulty")?.unwrap_or(U256::ZERO);
    let prevrandao = parse_b256_field(&result, "mixHash")
        .ok()
        .or(Some(B256::ZERO));

    Ok(BlockHeader {
        number,
        timestamp,
        coinbase,
        gas_limit,
        basefee,
        prevrandao,
        difficulty,
    })
}

fn parse_u64_field(value: &serde_json::Value, key: &str) -> Result<Option<u64>> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| {
            let s = s.strip_prefix("0x").unwrap_or(s);
            u64::from_str_radix(s, 16).with_context(|| format!("invalid {key} field"))
        })
        .transpose()
}

fn parse_u256_field(value: &serde_json::Value, key: &str) -> Result<Option<U256>> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| {
            let s = s.strip_prefix("0x").unwrap_or(s);
            U256::from_str_radix(s, 16).with_context(|| format!("invalid {key} field"))
        })
        .transpose()
}

fn parse_address_field(value: &serde_json::Value, key: &str) -> Result<Option<Address>> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.parse().with_context(|| format!("invalid {key} field")))
        .transpose()
}

fn parse_b256_field(value: &serde_json::Value, key: &str) -> Result<B256> {
    let s = value
        .get(key)
        .and_then(|v| v.as_str())
        .with_context(|| format!("missing {key} field"))?;
    s.parse().with_context(|| format!("invalid {key} field"))
}
