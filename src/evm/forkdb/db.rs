//! ForkDB: revm-native forked database backed by an RPC [`Client`].

use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use revm::{DatabaseRef, bytecode::Bytecode, primitives::KECCAK_EMPTY, state::AccountInfo};

use crate::evm::forkdb::client::Client;
use crate::evm::forkdb::error::Error;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;

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
    chain_id: u64,
}

impl ForkDB {
    pub fn new(client: Arc<Client>, block_number: u64, chain_id: u64) -> Self {
        Self {
            client,
            block_number,
            chain_id,
        }
    }

    /// Retry transient errors so that a batcher panic (or temporary RPC
    /// timeout / rate-limit) does not abort the current fuzzing run.
    fn with_retry<T>(&self, f: impl Fn() -> Result<T, Error>) -> Result<T, Error> {
        let mut last_err = None;
        for attempt in 0..3 {
            match f() {
                Ok(v) => return Ok(v),
                Err(e) if e.is_transient() => {
                    last_err = Some(e);
                    if attempt < 2 {
                        std::thread::sleep(std::time::Duration::from_millis(
                            50 * (1_u64 << attempt),
                        ));
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap())
    }

    /// Parse the heterogeneous batch responses for `basic_ref` into an
    /// `AccountInfo`.  The responses may arrive in any order; we match by
    /// variant rather than by index so that `db.rs` is decoupled from the
    /// batcher's ordering guarantees.
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
        self.with_retry(|| {
            // Fetch balance, nonce, and code in a single atomic batch so the
            // background worker can send them as one JSON-RPC batch request.
            let responses = self.client.request(&[
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
        })
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
        self.with_retry(|| {
            let mut responses = self.client.request(&[Request::GetStorageAt {
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
        })
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.with_retry(|| {
            let mut responses = self.client.request(&[Request::GetBlockByNumber {
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;
    use revm::database::CacheDB;
    use serde_json::json;

    use crate::evm::forkdb::{Config as ForkdbConfig, MockTransport, Transport};

    /// Regression: ForkDB must retry transient errors (including BatcherRestarted)
    /// instead of returning them directly to revm.
    #[test]
    fn forkdb_retries_transient_batcher_restarted() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Debug)]
        struct PanicOnceTransport {
            panics_remaining: AtomicUsize,
        }

        impl Transport for PanicOnceTransport {
            fn exec(
                &self,
                _url: &str,
                _payload: &serde_json::Value,
            ) -> anyhow::Result<serde_json::Value> {
                if self.panics_remaining.fetch_sub(1, Ordering::SeqCst) > 0 {
                    panic!("simulated transport panic");
                }
                // Return valid batch response for basic_ref (balance, nonce, code)
                Ok(json!([
                    {"jsonrpc":"2.0","id":0,"result":"0x1"},
                    {"jsonrpc":"2.0","id":1,"result":"0x2"},
                    {"jsonrpc":"2.0","id":2,"result":"0x6000"}
                ]))
            }
        }

        let transport = PanicOnceTransport {
            panics_remaining: AtomicUsize::new(1),
        };

        let config = ForkdbConfig::new("mock://panic").batch_timeout_ms(0).retries(0);
        let client = Client::new_with_transport(config, transport);
        let fork_db = ForkDB::new(Arc::new(client), 1, 1);

        let result = fork_db.basic_ref(Address::ZERO);
        assert!(
            result.is_ok(),
            "ForkDB must retry BatcherRestarted and eventually succeed, got: {result:?}"
        );
        let info = result.unwrap().unwrap();
        assert_eq!(info.balance, U256::from(1));
        assert_eq!(info.nonce, 2);
        let expected_code = Bytecode::new_raw(Bytes::from_static(&[0x60, 0x00]));
        assert_eq!(info.code, Some(expected_code.clone()));
        assert_eq!(info.code_hash, expected_code.hash_slow());
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
        let fork_db = ForkDB::new(Arc::new(client), 1, 1);

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

        let config = ForkdbConfig::new(url).batch_timeout_ms(0);
        let client = Client::new_with_transport(config, transport);
        let fork_db = ForkDB::new(Arc::new(client), 1, 1);

        let result = fork_db.block_hash_ref(1);
        assert!(
            result.is_err(),
            "block_hash_ref must error when RPC returns hash: null, got {result:?}"
        );
    }

    /// Regression: dropping all ForkDB (and CacheDB<ForkDB>) handles must
    /// release the underlying ClientInner so the supervisor thread exits.
    #[test]
    fn forkdb_drop_releases_client_inner() {
        let transport = MockTransport::default();
        let config = ForkdbConfig::new("mock://test").batch_timeout_ms(0);
        let client = Client::new_with_transport(config, transport);
        let weak = Arc::downgrade(&client.inner);

        let fork_db = ForkDB::new(Arc::new(client), 1, 1);
        let db = CacheDB::new(fork_db);
        let db2 = db.clone();

        drop(db);
        drop(db2);

        for _ in 0..40 {
            if weak.upgrade().is_none() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("ClientInner leaked through ForkDB/CacheDB clones");
    }
}
