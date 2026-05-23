//! ForkDB: revm-native forked database backed by an RPC [`Client`].

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use alloy_primitives::{Address, B256, U256};
use lru::LruCache;
use revm::{
    DatabaseRef, bytecode::Bytecode, database_interface::DBErrorMarker, primitives::KECCAK_EMPTY,
    state::AccountInfo,
};

use crate::evm::forkdb::client::Client;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;

const DEFAULT_CONTRACT_CACHE_CAPACITY: usize = 1024;

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
/// requests and keeps a small in-process LRU cache for contract bytecode
/// (needed for [`code_by_hash_ref`](DatabaseRef::code_by_hash_ref)).
#[derive(Clone, Debug)]
pub struct ForkDB {
    client: Arc<Client>,
    block_number: u64,
    /// Caches bytecode by code hash. `Mutex` is required because `LruCache`
    /// needs `&mut self` on both reads (to promote recency) and writes.
    contracts: Arc<Mutex<LruCache<B256, Bytecode>>>,
}

impl ForkDB {
    pub fn new(client: Arc<Client>, block_number: u64) -> Self {
        Self::with_capacity(client, block_number, DEFAULT_CONTRACT_CACHE_CAPACITY)
    }

    pub fn with_capacity(client: Arc<Client>, block_number: u64, cap: usize) -> Self {
        let contracts = match NonZeroUsize::new(cap) {
            Some(n) => LruCache::new(n),
            None => LruCache::unbounded(),
        };
        Self {
            client,
            block_number,
            contracts: Arc::new(Mutex::new(contracts)),
        }
    }

    /// Parse the heterogeneous batch responses for `basic_ref` into an
    /// `AccountInfo`.  The responses may arrive in any order; we match by
    /// variant rather than by index so that `db.rs` is decoupled from the
    /// batcher's ordering guarantees.
    fn parse_basic_responses(
        &self,
        responses: Vec<Response>,
    ) -> Result<Option<AccountInfo>, ForkDBError> {
        let mut balance = None;
        let mut nonce = None;
        let mut code = None;

        for response in responses {
            match response {
                Response::Balance(v) => {
                    if balance.is_some() {
                        return Err(ForkDBError::from(anyhow::anyhow!(
                            "duplicate Balance response"
                        )));
                    }
                    balance = Some(v);
                }
                Response::TransactionCount(v) => {
                    if nonce.is_some() {
                        return Err(ForkDBError::from(anyhow::anyhow!(
                            "duplicate TransactionCount response"
                        )));
                    }
                    nonce = Some(v);
                }
                Response::Code(v) => {
                    if code.is_some() {
                        return Err(ForkDBError::from(anyhow::anyhow!(
                            "duplicate Code response"
                        )));
                    }
                    code = Some(v);
                }
                _ => {
                    return Err(ForkDBError::from(anyhow::anyhow!(
                        "unexpected response in basic_ref batch"
                    )));
                }
            }
        }

        let balance = balance
            .ok_or_else(|| ForkDBError::from(anyhow::anyhow!("missing Balance response")))?;
        let nonce = nonce.ok_or_else(|| {
            ForkDBError::from(anyhow::anyhow!("missing TransactionCount response"))
        })?;
        let code =
            code.ok_or_else(|| ForkDBError::from(anyhow::anyhow!("missing Code response")))?;

        let bytecode = if code.is_empty() {
            Bytecode::default()
        } else {
            Bytecode::new_raw(code)
        };
        let code_hash = bytecode.hash_slow();
        if !bytecode.is_empty() {
            self.contracts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .put(code_hash, bytecode.clone());
        }

        Ok(Some(AccountInfo {
            balance,
            nonce,
            code_hash,
            code: Some(bytecode),
            account_id: None,
        }))
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

        self.parse_basic_responses(responses)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if code_hash == KECCAK_EMPTY || code_hash.is_zero() {
            return Ok(Bytecode::default());
        }
        match self
            .contracts
            .lock()
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;
    use serde_json::json;

    use crate::evm::forkdb::{Config as ForkdbConfig, MockTransport};

    /// Regression: ForkDB must keep its in-memory contracts cache bounded so
    /// that a long campaign touching thousands of unique contracts does not OOM.
    #[test]
    fn forkdb_contracts_cache_is_bounded() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let block_tag = json!("0x1");

        let addresses: Vec<Address> = (0..4u64)
            .map(|i| {
                let mut bytes = [0u8; 20];
                bytes[19] = (i + 1) as u8;
                Address::from(bytes)
            })
            .collect();

        for (i, addr) in addresses.iter().enumerate() {
            let payload = json!([
                {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":[json!(format!("0x{addr:x}")), block_tag.clone()]},
                {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":[json!(format!("0x{addr:x}")), block_tag.clone()]},
                {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":[json!(format!("0x{addr:x}")), block_tag.clone()]},
            ]);
            let code_hex = format!("0x{:04x}", i);
            transport.mock_response(
                url,
                &payload,
                json!([
                    {"jsonrpc":"2.0","id":0,"result":"0x0"},
                    {"jsonrpc":"2.0","id":1,"result":"0x0"},
                    {"jsonrpc":"2.0","id":2,"result": code_hex},
                ]),
            );
        }

        let config = ForkdbConfig::new(url);
        let client = Client::new_with_transport(config, transport.clone());
        let fork_db = ForkDB::with_capacity(Arc::new(client), 1, 2);

        for addr in &addresses {
            let _ = fork_db.basic_ref(*addr).unwrap().unwrap();
        }

        assert_eq!(
            fork_db
                .contracts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len(),
            2,
            "contracts cache must be bounded by LRU capacity"
        );
    }

    /// Regression: ForkDB::basic_ref must not assume that the batcher returns
    /// responses in the same order as the requests.  If the response order
    /// changes (or a future batcher reorders them), matching by index silently
    /// corrupts account state.
    #[test]
    fn basic_ref_is_order_independent() {
        let transport = MockTransport::default();
        let config = ForkdbConfig::new("mock://test");
        let client = Client::new_with_transport(config, transport);
        let fork_db = ForkDB::new(Arc::new(client), 1);

        // Responses arrive in a different order than the requests.
        let responses = vec![
            Response::TransactionCount(2),
            Response::Code(Bytes::from_static(&[0x60, 0x00])),
            Response::Balance(U256::from(1)),
        ];

        let info = fork_db.parse_basic_responses(responses).unwrap().unwrap();
        assert_eq!(info.balance, U256::from(1));
        assert_eq!(info.nonce, 2);
        let expected_code = Bytecode::new_raw(Bytes::from_static(&[0x60, 0x00]));
        assert_eq!(info.code, Some(expected_code.clone()));
        assert_eq!(info.code_hash, expected_code.hash_slow());
    }
}
