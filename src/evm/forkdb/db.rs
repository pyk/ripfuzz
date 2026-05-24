//! ForkDB: revm-native forked database backed by a [`SharedBackend`].

use alloy_primitives::{Address, B256, U256};
use revm::{DatabaseRef, bytecode::Bytecode, primitives::KECCAK_EMPTY, state::AccountInfo};

use crate::evm::forkdb::backend::SharedBackend;
use crate::evm::forkdb::error::Error;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;

/// Remote backend that satisfies `DatabaseRef`.
///
/// All RPC state fetching is delegated to the internal [`SharedBackend`],
/// which handles caching, deduplication, rate limiting, retries, and
/// automatic batching. This struct only maps revm database operations to
/// typed RPC requests. Bytecode is returned inline with `basic_ref` so revm
/// never needs to call `code_by_hash_ref` during normal execution.
#[derive(Clone, Debug)]
pub struct ForkDB {
    backend: SharedBackend,
    block_number: u64,
    chain_id: u64,
}

impl ForkDB {
    pub fn new(backend: SharedBackend, block_number: u64, chain_id: u64) -> Self {
        Self {
            backend,
            block_number,
            chain_id,
        }
    }

    /// Parse the heterogeneous batch responses for `basic_ref` into an
    /// `AccountInfo`.  The responses may arrive in any order; we match by
    /// variant rather than by index so that `db.rs` is decoupled from the
    /// backend's ordering guarantees.
    fn parse_basic_responses(
        &self,
        responses: Vec<Response>,
    ) -> Result<Option<AccountInfo>, Error> {
        let mut balance = None;
        let mut nonce = None;
        let mut code = None;

        for response in responses {
            match response {
                Response::Balance(v) => {
                    if balance.is_some() {
                        return Err(Error::UnexpectedResponse {
                            message: "duplicate Balance response".into(),
                        });
                    }
                    balance = Some(v);
                }
                Response::TransactionCount(v) => {
                    if nonce.is_some() {
                        return Err(Error::UnexpectedResponse {
                            message: "duplicate TransactionCount response".into(),
                        });
                    }
                    nonce = Some(v);
                }
                Response::Code(v) => {
                    if code.is_some() {
                        return Err(Error::UnexpectedResponse {
                            message: "duplicate Code response".into(),
                        });
                    }
                    code = Some(v);
                }
                _ => {
                    return Err(Error::UnexpectedResponse {
                        message: "unexpected response in basic_ref batch".into(),
                    });
                }
            }
        }

        let balance = balance.ok_or_else(|| Error::UnexpectedResponse {
            message: "missing Balance response".into(),
        })?;
        let nonce = nonce.ok_or_else(|| Error::UnexpectedResponse {
            message: "missing TransactionCount response".into(),
        })?;
        let code = code.ok_or_else(|| Error::UnexpectedResponse {
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
    type Error = Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // Fetch balance, nonce, and code in a single atomic batch so the
        // backend can send them as one JSON-RPC batch request.
        let responses = self.backend.fetch_or_wait(&[
            Request::GetBalance {
                chain_id: self.chain_id,
                address,
                block: self.block_number,
            },
            Request::GetTransactionCount {
                chain_id: self.chain_id,
                address,
                block: self.block_number,
            },
            Request::GetCode {
                chain_id: self.chain_id,
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
        Err(Error::MissingAccount {
            message: format!(
                "code hash {code_hash} not present in fork DB; code should have been loaded via basic_ref"
            ),
        })
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let mut responses = self.backend.fetch_or_wait(&[Request::GetStorageAt {
            chain_id: self.chain_id,
            address,
            slot: index,
            block: self.block_number,
        }])?;
        let response = responses.pop().ok_or_else(|| Error::UnexpectedResponse {
            message: "expected one response for GetStorageAt".into(),
        })?;

        match response {
            Response::StorageAt(v) => Ok(v),
            _ => Err(Error::UnexpectedResponse {
                message: "unexpected response for GetStorageAt".into(),
            }),
        }
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let mut responses = self.backend.fetch_or_wait(&[Request::GetBlockByNumber {
            chain_id: self.chain_id,
            block: number,
            full_tx: false,
        }])?;
        let response = responses.pop().ok_or_else(|| Error::UnexpectedResponse {
            message: "expected one response for GetBlockByNumber".into(),
        })?;

        match response {
            Response::BlockByNumber(b) => b.hash.ok_or_else(|| Error::UnexpectedResponse {
                message: format!("block {number}: hash missing in GetBlockByNumber response"),
            }),
            _ => Err(Error::UnexpectedResponse {
                message: "unexpected response for GetBlockByNumber".into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use alloy_primitives::Bytes;
    use serde_json::json;

    use crate::evm::forkdb::{Config as ForkdbConfig, MockTransport, Transport};

    /// Regression: ForkDB must NOT sleep the fuzzer thread when the backend
    /// returns RpcTimeout. The backend is the sole retry layer; ForkDB should
    /// propagate the error immediately.
    #[test]
    fn forkdb_does_not_sleep_on_rpc_timeout() {
        #[derive(Debug)]
        struct TimeoutTransport;

        impl Transport for TimeoutTransport {
            fn exec(&self, _url: &str, _payload: &serde_json::Value) -> Result<serde_json::Value> {
                Err(anyhow::anyhow!("request timed out"))
            }
        }

        let config = ForkdbConfig::new("mock://timeout")
            .batch_timeout_ms(0)
            .retries(0);
        let backend = SharedBackend::new_with_transport(config, TimeoutTransport);
        let fork_db = ForkDB::new(backend, 1, 1);

        let start = std::time::Instant::now();
        let result = fork_db.basic_ref(Address::ZERO);
        let elapsed = start.elapsed();

        assert!(
            matches!(result, Err(Error::RpcTimeout { .. })),
            "ForkDB must propagate RpcTimeout immediately, got: {result:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(30),
            "ForkDB must not sleep on RpcTimeout; took {elapsed:?}"
        );
    }

    /// Regression: ForkDB must NOT sleep the fuzzer thread when the backend
    /// returns RateLimited. The backend is the sole retry layer; ForkDB should
    /// propagate the error immediately.
    #[test]
    fn forkdb_does_not_sleep_on_rate_limited() {
        #[derive(Debug)]
        struct RateLimitTransport;

        impl Transport for RateLimitTransport {
            fn exec(&self, _url: &str, _payload: &serde_json::Value) -> Result<serde_json::Value> {
                Err(anyhow::anyhow!("429 too many requests"))
            }
        }

        let config = ForkdbConfig::new("mock://ratelimit")
            .batch_timeout_ms(0)
            .retries(0);
        let backend = SharedBackend::new_with_transport(config, RateLimitTransport);
        let fork_db = ForkDB::new(backend, 1, 1);

        let start = std::time::Instant::now();
        let result = fork_db.basic_ref(Address::ZERO);
        let elapsed = start.elapsed();

        assert!(
            matches!(result, Err(Error::RateLimited { .. })),
            "ForkDB must propagate RateLimited immediately, got: {result:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(30),
            "ForkDB must not sleep on RateLimited; took {elapsed:?}"
        );
    }

    /// Regression: ForkDB::basic_ref must not assume that the backend returns
    /// responses in the same order as the requests.  If the response order
    /// changes (or a future backend reorders them), matching by index silently
    /// corrupts account state.
    #[test]
    fn basic_ref_is_order_independent() {
        let transport = MockTransport::default();
        let config = ForkdbConfig::new("mock://test");
        let backend = SharedBackend::new_with_transport(config, transport);
        let fork_db = ForkDB::new(backend, 1, 1);

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

    /// Regression: Error must be an enumerated type so callers can
    /// programmatically distinguish between transient and permanent failures.
    #[test]
    fn forkdb_error_variants_are_programmatically_distinguishable() {
        let timeout = Error::RpcTimeout {
            url: "http://rpc.example".into(),
        };
        assert!(matches!(timeout, Error::RpcTimeout { .. }));
        assert!(timeout.is_transient());

        let rate_limited = Error::RateLimited {
            url: "http://rpc.example".into(),
        };
        assert!(matches!(rate_limited, Error::RateLimited { .. }));
        assert!(rate_limited.is_transient());

        let rpc_err = Error::RpcError {
            code: -32000,
            message: "bad block".into(),
        };
        assert!(matches!(rpc_err, Error::RpcError { .. }));
        assert!(!rpc_err.is_transient());

        let decode = Error::DecodeError {
            message: "invalid json".into(),
        };
        assert!(matches!(decode, Error::DecodeError { .. }));
        assert!(!decode.is_transient());
    }

    /// Regression: ForkDB::block_hash_ref must not return B256::default()
    /// when `eth_getBlockByNumber` returns a block with `"hash": null`.
    /// Returning the zero hash makes the BLOCKHASH opcode deterministic and
    /// breaks property invariants that assume a non-zero, non-predictable
    /// value. A missing hash must propagate as an error.
    #[test]
    fn block_hash_ref_rejects_missing_hash() {
        let transport = MockTransport::default();
        let url = "mock://test";

        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":["0x1",false]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([{
                "jsonrpc":"2.0",
                "id":0,
                "result":{
                    "number":"0x1",
                    "timestamp":"0x0",
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0x0",
                    "hash":null
                }
            }]),
        );

        let config = ForkdbConfig::new(url).batch_timeout_ms(0).batch_size(1);
        let backend = SharedBackend::new_with_transport(config, transport);
        let fork_db = ForkDB::new(backend, 1, 1);

        let result = fork_db.block_hash_ref(1);
        assert!(
            result.is_err(),
            "block_hash_ref must error when RPC returns hash: null, got {result:?}"
        );
    }

    /// Regression: ForkDB must store SharedBackend directly, not
    /// Arc<SharedBackend>. SharedBackend is already internally
    /// reference-counted (Arc<SharedBackendInner>), so wrapping it in another
    /// Arc wastes an extra heap allocation.
    #[test]
    fn forkdb_stores_backend_directly() {
        let transport = MockTransport::default();
        let config = ForkdbConfig::new("mock://test");
        let backend = SharedBackend::new_with_transport(config, transport);

        // Must be possible to construct ForkDB from a plain SharedBackend
        // without wrapping in Arc. This proves we are not double-wrapping.
        let fork_db = ForkDB::new(backend.clone(), 1, 1);

        // Clone must be cheap because SharedBackend::clone only increments the
        // inner Arc refcount.
        let _fork_db2 = fork_db.clone();
    }
}
