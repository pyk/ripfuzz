//! ForkDB: revm-native forked database backed by an RPC [`Client`].

use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use revm::{
    DatabaseRef, bytecode::Bytecode, database_interface::DBErrorMarker, primitives::KECCAK_EMPTY,
    state::AccountInfo,
};

use crate::evm::forkdb::client::Client;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;

/// Enumerated error type for ForkDB so callers can programmatically
/// distinguish between transient and permanent failures.
#[derive(Debug, Clone)]
pub enum ForkDBError {
    /// RPC transport timed out (e.g., HTTP request exceeded deadline).
    RpcTimeout { url: String },
    /// Rate limited by the RPC provider (HTTP 429 or similar).
    RateLimited { url: String },
    /// Failed to serialize or deserialize JSON.
    DecodeError { message: String },
    /// The RPC server returned a JSON-RPC error object.
    RpcError { code: i64, message: String },
    /// An unexpected response was received (duplicate, missing, wrong variant).
    UnexpectedResponse { message: String },
    /// The requested account or code hash is not present in the fork database.
    MissingAccount { message: String },
    /// An internal error (channel closed, worker shut down, etc.).
    Internal { message: String },
}

impl ForkDBError {
    /// Returns `true` if this error is likely transient and the request
    /// should be retried.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::RpcTimeout { .. } | Self::RateLimited { .. })
    }
}

impl std::fmt::Display for ForkDBError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RpcTimeout { url } => write!(f, "RPC timeout: {url}"),
            Self::RateLimited { url } => write!(f, "RPC rate limited: {url}"),
            Self::DecodeError { message } => write!(f, "RPC decode error: {message}"),
            Self::RpcError { code, message } => write!(f, "RPC error {code}: {message}"),
            Self::UnexpectedResponse { message } => {
                write!(f, "unexpected RPC response: {message}")
            }
            Self::MissingAccount { message } => {
                write!(f, "missing account in fork DB: {message}")
            }
            Self::Internal { message } => write!(f, "internal fork DB error: {message}"),
        }
    }
}

impl std::error::Error for ForkDBError {}

impl DBErrorMarker for ForkDBError {}

impl From<anyhow::Error> for ForkDBError {
    fn from(e: anyhow::Error) -> Self {
        let msg = format!("{e}");
        // Classify transport errors by inspecting the root cause.
        if let Some(ureq_err) = e.root_cause().downcast_ref::<ureq::Error>() {
            match ureq_err {
                ureq::Error::StatusCode(429) => {
                    return Self::RateLimited { url: String::new() };
                }
                ureq::Error::Timeout(_) => {
                    return Self::RpcTimeout { url: String::new() };
                }
                _ => {}
            }
        }
        if let Some(json_err) = e.root_cause().downcast_ref::<serde_json::Error>() {
            return Self::DecodeError {
                message: format!("{json_err}"),
            };
        }
        if msg.contains("429") || msg.contains("rate limit") || msg.contains("too many requests") {
            return Self::RateLimited { url: String::new() };
        }
        if msg.contains("timeout") || msg.contains("timed out") {
            return Self::RpcTimeout { url: String::new() };
        }
        Self::Internal { message: msg }
    }
}

/// Remote backend that satisfies `DatabaseRef`.
///
/// All RPC state fetching is delegated to the internal [`Client`], which
/// handles caching, deduplication, rate limiting, retries, and automatic
/// batching. This struct only maps revm database operations to typed RPC
/// requests. Bytecode is returned inline with `basic_ref` so revm never
/// needs to call `code_by_hash_ref` during normal execution.
#[derive(Clone, Debug)]
pub struct ForkDB {
    client: Arc<Client>,
    block_number: u64,
}

impl ForkDB {
    pub fn new(client: Arc<Client>, block_number: u64) -> Self {
        Self {
            client,
            block_number,
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
                        return Err(ForkDBError::UnexpectedResponse {
                            message: "duplicate Balance response".into(),
                        });
                    }
                    balance = Some(v);
                }
                Response::TransactionCount(v) => {
                    if nonce.is_some() {
                        return Err(ForkDBError::UnexpectedResponse {
                            message: "duplicate TransactionCount response".into(),
                        });
                    }
                    nonce = Some(v);
                }
                Response::Code(v) => {
                    if code.is_some() {
                        return Err(ForkDBError::UnexpectedResponse {
                            message: "duplicate Code response".into(),
                        });
                    }
                    code = Some(v);
                }
                _ => {
                    return Err(ForkDBError::UnexpectedResponse {
                        message: "unexpected response in basic_ref batch".into(),
                    });
                }
            }
        }

        let balance = balance.ok_or_else(|| ForkDBError::UnexpectedResponse {
            message: "missing Balance response".into(),
        })?;
        let nonce = nonce.ok_or_else(|| ForkDBError::UnexpectedResponse {
            message: "missing TransactionCount response".into(),
        })?;
        let code = code.ok_or_else(|| ForkDBError::UnexpectedResponse {
            message: "missing Code response".into(),
        })?;

        let bytecode = if code.is_empty() {
            Bytecode::default()
        } else {
            Bytecode::new_raw(code)
        };
        let code_hash = bytecode.hash_slow();

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
        let responses = self.client.request(&[
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
        ])?;

        self.parse_basic_responses(responses)
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if code_hash == KECCAK_EMPTY || code_hash.is_zero() {
            return Ok(Bytecode::default());
        }
        // revm's CacheDB caches code loaded via basic_ref, so this path is
        // rarely exercised in practice. AlloyDB takes the same stance and
        // panics here; we return a typed error instead.
        Err(ForkDBError::MissingAccount {
            message: format!(
                "code hash {code_hash} not present in fork DB; code should have been loaded via basic_ref"
            ),
        })
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let mut responses = self.client.request(&[Request::GetStorageAt {
            address,
            slot: index,
            block: self.block_number,
        }])?;
        let response = responses
            .pop()
            .ok_or_else(|| ForkDBError::UnexpectedResponse {
                message: "expected one response for GetStorageAt".into(),
            })?;

        match response {
            Response::StorageAt(v) => Ok(v),
            _ => Err(ForkDBError::UnexpectedResponse {
                message: "unexpected response for GetStorageAt".into(),
            }),
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let mut responses = self.client.request(&[Request::GetBlockByNumber {
            block: number,
            full_tx: false,
        }])?;
        let response = responses
            .pop()
            .ok_or_else(|| ForkDBError::UnexpectedResponse {
                message: "expected one response for GetBlockByNumber".into(),
            })?;

        match response {
            Response::BlockByNumber(b) => Ok(b.hash.unwrap_or_default()),
            _ => Err(ForkDBError::UnexpectedResponse {
                message: "unexpected response for GetBlockByNumber".into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;
    use serde_json::json;

    use crate::evm::forkdb::{Config as ForkdbConfig, MockTransport};

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

    /// Regression: ForkDBError must be an enumerated type so callers can
    /// programmatically distinguish between transient and permanent failures.
    #[test]
    fn forkdb_error_variants_are_programmatically_distinguishable() {
        let timeout = ForkDBError::RpcTimeout {
            url: "http://rpc.example".into(),
        };
        assert!(matches!(timeout, ForkDBError::RpcTimeout { .. }));
        assert!(timeout.is_transient());

        let rate_limited = ForkDBError::RateLimited {
            url: "http://rpc.example".into(),
        };
        assert!(matches!(rate_limited, ForkDBError::RateLimited { .. }));
        assert!(rate_limited.is_transient());

        let rpc_err = ForkDBError::RpcError {
            code: -32000,
            message: "bad block".into(),
        };
        assert!(matches!(rpc_err, ForkDBError::RpcError { .. }));
        assert!(!rpc_err.is_transient());

        let decode = ForkDBError::DecodeError {
            message: "invalid json".into(),
        };
        assert!(matches!(decode, ForkDBError::DecodeError { .. }));
        assert!(!decode.is_transient());
    }
}
