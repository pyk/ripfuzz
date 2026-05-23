//! ForkDB: revm-native forked database backed by an RPC [`Client`].

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use alloy_primitives::{Address, B256, U256};
use revm::{
    DatabaseRef, bytecode::Bytecode, database_interface::DBErrorMarker, primitives::KECCAK_EMPTY,
    state::AccountInfo,
};

use crate::evm::forkdb::client::Client;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;

/// Thin newtype around `anyhow::Error` so we can implement `DBErrorMarker`.
#[derive(Debug)]
pub struct ForkDBError(anyhow::Error);

impl std::fmt::Display for ForkDBError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ForkDBError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl DBErrorMarker for ForkDBError {}

impl From<anyhow::Error> for ForkDBError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}

/// Remote backend that satisfies `DatabaseRef`.
///
/// All RPC state fetching is delegated to the internal [`Client`], which
/// handles caching, deduplication, rate limiting, retries, and automatic
/// batching. This struct only maps revm database operations to typed RPC
/// requests and keeps a small in-process cache for contract bytecode (needed
/// for [`code_by_hash_ref`](DatabaseRef::code_by_hash_ref)).
#[derive(Clone, Debug)]
pub struct ForkDB {
    client: Arc<Client>,
    block_number: u64,
    /// Caches bytecode by code hash. `RwLock` chosen because
    /// `code_by_hash_ref` is read-heavy while writes only happen on cache
    /// misses during `basic_ref`.
    contracts: Arc<RwLock<HashMap<B256, Bytecode>>>,
}

impl ForkDB {
    pub fn new(client: Arc<Client>, block_number: u64) -> Self {
        Self {
            client,
            block_number,
            contracts: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl DatabaseRef for ForkDB {
    type Error = ForkDBError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // Fetch balance, nonce, and code in a single atomic batch so the
        // background worker can send them as one JSON-RPC batch request.
        let responses = self
            .client
            .request(&[
                Request::GetBalance {
                    address,
                    block: self.block_number,
                },
                Request::GetTransactionCount {
                    address,
                    block: self.block_number,
                },
                Request::GetCode {
                    address,
                    block: self.block_number,
                },
            ])
            .map_err(ForkDBError::from)?;

        let balance = match &responses[0] {
            Response::Balance(v) => *v,
            _ => {
                return Err(ForkDBError::from(anyhow::anyhow!(
                    "unexpected response for GetBalance"
                )));
            }
        };
        let nonce = match &responses[1] {
            Response::TransactionCount(v) => *v,
            _ => {
                return Err(ForkDBError::from(anyhow::anyhow!(
                    "unexpected response for GetTransactionCount"
                )));
            }
        };
        let code = match &responses[2] {
            Response::Code(v) => v.clone(),
            _ => {
                return Err(ForkDBError::from(anyhow::anyhow!(
                    "unexpected response for GetCode"
                )));
            }
        };

        let bytecode = if code.is_empty() {
            Bytecode::default()
        } else {
            Bytecode::new_raw(code)
        };
        let code_hash = bytecode.hash_slow();
        if !bytecode.is_empty() {
            self.contracts
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(code_hash, bytecode.clone());
        }

        Ok(Some(AccountInfo {
            balance,
            nonce,
            code_hash,
            code: Some(bytecode),
            account_id: None,
        }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if code_hash == KECCAK_EMPTY || code_hash.is_zero() {
            return Ok(Bytecode::default());
        }
        match self
            .contracts
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&code_hash)
        {
            Some(code) => Ok(code.clone()),
            None => Err(ForkDBError::from(anyhow::anyhow!(
                "code hash {code_hash} not found in fork database"
            ))),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let mut responses = self
            .client
            .request(&[Request::GetStorageAt {
                address,
                slot: index,
                block: self.block_number,
            }])
            .map_err(ForkDBError::from)?;
        let response = responses.pop().ok_or_else(|| {
            ForkDBError::from(anyhow::anyhow!("expected one response for GetStorageAt"))
        })?;

        match response {
            Response::StorageAt(v) => Ok(v),
            _ => Err(ForkDBError::from(anyhow::anyhow!(
                "unexpected response for GetStorageAt"
            ))),
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let mut responses = self
            .client
            .request(&[Request::GetBlockByNumber {
                block: number,
                full_tx: false,
            }])
            .map_err(ForkDBError::from)?;
        let response = responses.pop().ok_or_else(|| {
            ForkDBError::from(anyhow::anyhow!(
                "expected one response for GetBlockByNumber"
            ))
        })?;

        match response {
            Response::BlockByNumber(b) => Ok(b.hash.unwrap_or_default()),
            _ => Err(ForkDBError::from(anyhow::anyhow!(
                "unexpected response for GetBlockByNumber"
            ))),
        }
    }
}
