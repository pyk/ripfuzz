//! JSON-RPC client with per-request caching, deduplication, rate limiting,
//! retries, and automatic batching.
//!
//! Exposes exactly four high-level typed methods that map to the individual
//! RPC requests needed by the ForkDB.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use alloy_primitives::{Address, U256};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use tracing::{instrument, trace};
use ureq::Agent;

use crate::evm::forkdb::rpc::batcher::Batcher;
use crate::evm::forkdb::rpc::cache::Cache;
use crate::evm::forkdb::rpc::config::Config;
use crate::evm::forkdb::rpc::dedup::DedupTable;
use crate::evm::forkdb::rpc::limiter::RateLimiter;
use crate::evm::forkdb::rpc::transport::Transport;
use crate::evm::forkdb::rpc::types::{
    GetBlockByNumberResponse, RemoteAccountInfo, RemoteBlockInfo, RemoteChainInfo, RpcRequest,
};

#[derive(Clone, Debug)]
pub struct Client {
    inner: Arc<ClientInner>,
}

#[derive(Debug)]
struct ClientInner {
    _url: String,
    _retries: u32,
    _backoff: Duration,
    cache: Option<Cache>,
    dedup: DedupTable,
    batcher: Batcher,
    next_id: AtomicU64,
}

impl Client {
    /// Create a new client from a validated configuration.
    pub fn new(config: Config) -> Self {
        let timeout = Duration::from_millis(config.timeout_ms);
        let agent_cfg = Agent::config_builder()
            .timeout_global(Some(timeout))
            .build();
        let agent = Agent::new_with_config(agent_cfg);
        Self::new_with_transport(config, agent)
    }

    /// Create a new client with a custom transport (e.g. for testing).
    pub fn new_with_transport(config: Config, transport: impl Transport + 'static) -> Self {
        let limiter = config.rate_limit.map(RateLimiter::new);
        let cache = config.cache_dir.map(Cache::new);
        let batcher = Batcher::new(
            Arc::new(transport),
            config.url.clone(),
            config.retries,
            Duration::from_millis(config.backoff_ms),
            limiter.map(Arc::new),
            config.batch_size,
            Duration::from_millis(config.batch_timeout_ms),
        );
        Self {
            inner: Arc::new(ClientInner {
                _url: config.url,
                _retries: config.retries,
                _backoff: Duration::from_millis(config.backoff_ms),
                cache,
                dedup: DedupTable::new(),
                batcher,
                next_id: AtomicU64::new(1),
            }),
        }
    }

    // -----------------------------------------------------------------
    // Public high-level API
    // -----------------------------------------------------------------

    /// Fetch chain ID and block header for a specific block.
    #[instrument(skip(self))]
    pub fn get_remote_chain_info(&self, block: u64) -> Result<RemoteChainInfo> {
        let requests = vec![RpcRequest::ChainId, RpcRequest::GetBlockByNumber { block }];
        let responses = self.dispatch_batch(requests)?;
        let chain_id = parse_chain_id(&responses[0])?;
        let block_info = parse_block(&responses[1])?;
        Ok(RemoteChainInfo {
            chain_id,
            block: block_info,
        })
    }

    /// Fetch balance, nonce, and code for an address at a specific block.
    #[instrument(skip(self))]
    pub fn get_remote_account_info(
        &self,
        address: Address,
        block: u64,
    ) -> Result<RemoteAccountInfo> {
        let requests = vec![
            RpcRequest::GetBalance { address, block },
            RpcRequest::GetTransactionCount { address, block },
            RpcRequest::GetCode { address, block },
        ];
        let responses = self.dispatch_batch(requests)?;
        let balance = parse_balance(&responses[0])?;
        let nonce = parse_nonce(&responses[1])?;
        let code = parse_code(&responses[2])?;
        Ok(RemoteAccountInfo {
            balance,
            nonce,
            code,
        })
    }

    /// Fetch a block header by number.
    #[instrument(skip(self))]
    pub fn get_remote_block_info(&self, block: u64) -> Result<RemoteBlockInfo> {
        let requests = vec![RpcRequest::GetBlockByNumber { block }];
        let responses = self.dispatch_batch(requests)?;
        parse_block(&responses[0])
    }

    /// Fetch a storage slot for an address at a specific block.
    #[instrument(skip(self))]
    pub fn get_remote_storage_info(
        &self,
        address: Address,
        slot: U256,
        block: u64,
    ) -> Result<U256> {
        let requests = vec![RpcRequest::GetStorageAt {
            address,
            slot,
            block,
        }];
        let responses = self.dispatch_batch(requests)?;
        parse_storage(&responses[0])
    }

    // -----------------------------------------------------------------
    // Internal call pipeline
    // -----------------------------------------------------------------

    /// Unified internal dispatch that handles caching, deduplication,
    /// batching, retries, and transport for one or more requests.
    ///
    /// Requests are submitted together so the batcher can group them with
    /// concurrent requests from other threads.
    fn dispatch_batch(&self, requests: Vec<RpcRequest>) -> Result<Vec<Value>> {
        let mut cached: Vec<(usize, Result<Value>)> = Vec::with_capacity(requests.len());
        let mut to_fetch = Vec::new();

        // 1. Cache and dedup checks
        for (idx, req) in requests.into_iter().enumerate() {
            let cache_key = req.cache_key();

            if let Some(ref cache) = self.inner.cache
                && let Some(envelope) = cache.get(&req)
            {
                trace!(%cache_key, "cache hit");
                cached.push((idx, Ok(envelope)));
                continue;
            }

            if let Some(result) = self.inner.dedup.register(&cache_key) {
                trace!(%cache_key, "dedup hit");
                cached.push((idx, result));
                continue;
            }

            let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
            let rx = self.inner.batcher.submit(id, &req);
            to_fetch.push((idx, cache_key, req, rx));
        }

        // 2. Wait for network responses
        let mut fetched: Vec<(usize, Result<Value>)> = Vec::with_capacity(to_fetch.len());
        for (idx, cache_key, req, rx) in to_fetch {
            let guard = self.inner.dedup.guard(&cache_key);
            let result = match rx.recv_timeout(Duration::from_secs(30)) {
                Ok(Ok(v)) => Ok(v),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(anyhow::anyhow!(
                    "batch request timed out or batch thread died"
                )),
            };

            if let Ok(ref envelope) = result
                && let Some(ref cache) = self.inner.cache
            {
                cache.insert(&req, envelope);
            }
            self.inner.dedup.complete(&cache_key, result.as_ref());
            guard.deactivate();
            fetched.push((idx, result));
        }

        cached.extend(fetched);
        cached.sort_by_key(|(idx, _)| *idx);
        cached.into_iter().map(|(_, r)| r).collect()
    }
}

// -----------------------------------------------------------------
// Parsers
// -----------------------------------------------------------------

fn parse_chain_id(value: &Value) -> Result<u64> {
    let result = value
        .get("result")
        .and_then(|v| v.as_str())
        .context("missing result field in eth_chainId response")?;
    let hex = result.strip_prefix("0x").unwrap_or(result);
    u64::from_str_radix(hex, 16).context("invalid chain id hex")
}

fn parse_block(value: &Value) -> Result<RemoteBlockInfo> {
    let result = value
        .get("result")
        .cloned()
        .context("missing result field in eth_getBlockByNumber response")?;
    if result.is_null() {
        bail!("block not found");
    }
    let response: GetBlockByNumberResponse =
        serde_json::from_value(result).context("invalid block response")?;
    Ok(response.into())
}

fn parse_balance(value: &Value) -> Result<U256> {
    let result = value
        .get("result")
        .and_then(|v| v.as_str())
        .context("missing result field in eth_getBalance response")?;
    result.parse().context("invalid balance hex")
}

fn parse_nonce(value: &Value) -> Result<u64> {
    let result = value
        .get("result")
        .and_then(|v| v.as_str())
        .context("missing result field in eth_getTransactionCount response")?;
    let hex = result.strip_prefix("0x").unwrap_or(result);
    u64::from_str_radix(hex, 16).context("invalid nonce hex")
}

fn parse_code(value: &Value) -> Result<alloy_primitives::Bytes> {
    let result = value
        .get("result")
        .and_then(|v| v.as_str())
        .context("missing result field in eth_getCode response")?;
    result.parse().context("invalid code hex")
}

fn parse_storage(value: &Value) -> Result<U256> {
    let result = value
        .get("result")
        .and_then(|v| v.as_str())
        .context("missing result field in eth_getStorageAt response")?;
    result.parse().context("invalid storage hex")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use alloy_primitives::{Address, U256};
    use serde_json::json;

    use super::*;
    use crate::evm::forkdb::rpc::transport::MockTransport;

    #[test]
    fn mock_transport_roundtrip() {
        let transport = MockTransport::default();
        let payload_chain = RpcRequest::ChainId.to_json_payload(1);
        let payload_block = RpcRequest::GetBlockByNumber { block: 1 }.to_json_payload(2);
        transport.mock_response(
            "mock://test",
            &payload_chain,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );
        transport.mock_response(
            "mock://test",
            &payload_block,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "number": "0x1",
                    "timestamp": "0x1",
                    "miner": "0x0000000000000000000000000000000000000000",
                    "gasLimit": "0xffffffffffffffff",
                    "baseFeePerGas": "0x0",
                    "difficulty": "0x0",
                    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
                }
            }),
        );

        let config = Config::new("mock://test").batch_size(1);
        let rpc = Client::new_with_transport(config, transport.clone());

        let result = rpc.get_remote_chain_info(1).unwrap();
        assert_eq!(result.chain_id, 0x1a2b);
    }

    #[test]
    fn dedup_coalesces_parallel_requests() {
        let transport = MockTransport::default();
        transport.set_delay(Duration::from_millis(100));
        let payload_chain = RpcRequest::ChainId.to_json_payload(1);
        let payload_block = RpcRequest::GetBlockByNumber { block: 1 }.to_json_payload(2);
        transport.mock_response(
            "mock://test",
            &payload_chain,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );
        transport.mock_response(
            "mock://test",
            &payload_block,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "number": "0x1",
                    "timestamp": "0x1",
                    "miner": "0x0000000000000000000000000000000000000000",
                    "gasLimit": "0xffffffffffffffff",
                    "baseFeePerGas": "0x0",
                    "difficulty": "0x0",
                    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
                }
            }),
        );

        let config = Config::new("mock://test").batch_size(1);
        let rpc = Client::new_with_transport(config, transport.clone());
        let rpc2 = rpc.clone();

        let t1 = std::thread::spawn(move || rpc.get_remote_chain_info(1));
        let t2 = std::thread::spawn(move || rpc2.get_remote_chain_info(1));

        let r1 = t1.join().unwrap().unwrap();
        let r2 = t2.join().unwrap().unwrap();
        assert_eq!(r1.chain_id, r2.chain_id);
        assert_eq!(r1.chain_id, 0x1a2b);
        assert_eq!(transport.call_count("mock://test", &payload_chain), 1);
    }

    #[test]
    fn rate_limit_throttles_without_network() {
        let transport = MockTransport::default();
        // get_remote_chain_info consumes two ids per call; mock 4 calls worth.
        for i in 0..4 {
            let chain_id = i * 2 + 1;
            let block_id = i * 2 + 2;
            let payload_chain = RpcRequest::ChainId.to_json_payload(chain_id);
            let payload_block = RpcRequest::GetBlockByNumber { block: 1 }.to_json_payload(block_id);
            transport.mock_response(
                "mock://test",
                &payload_chain,
                json!({"jsonrpc": "2.0", "id": chain_id, "result": "0x1"}),
            );
            transport.mock_response(
                "mock://test",
                &payload_block,
                json!({
                    "jsonrpc": "2.0",
                    "id": block_id,
                    "result": {
                        "number": "0x1",
                        "timestamp": "0x1",
                        "miner": "0x0000000000000000000000000000000000000000",
                        "gasLimit": "0xffffffffffffffff",
                        "baseFeePerGas": "0x0",
                        "difficulty": "0x0",
                        "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
                    }
                }),
            );
        }

        let config = Config::new("mock://test").rate_limit(Some(2)).batch_size(1);
        let rpc = Client::new_with_transport(config, transport);

        let t0 = std::time::Instant::now();
        for _ in 0..4 {
            let _ = rpc.get_remote_chain_info(1).unwrap();
        }
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() >= 800,
            "rate limit did not throttle: {elapsed:?}"
        );
    }

    /// Regression test: a JSON-RPC batch is dispatched as a single HTTP
    /// POST, so it must consume exactly one rate-limit token.
    #[test]
    fn rate_limit_batch_counts_as_single_request() {
        let transport = MockTransport::default();
        let addr = json!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        let block_tag = json!("0x1");
        let batch = json!([
            {"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":[addr, block_tag]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getTransactionCount","params":[addr, block_tag]},
            {"jsonrpc":"2.0","id":3,"method":"eth_getCode","params":[addr, block_tag]},
        ]);
        transport.mock_response(
            "mock://test",
            &batch,
            json!([
                {"jsonrpc":"2.0","id":1,"result":"0x4ec7cefe1a0664fd"},
                {"jsonrpc":"2.0","id":2,"result":"0x1707"},
                {"jsonrpc":"2.0","id":3,"result":"0x6060604052"},
            ]),
        );

        let config = Config::new("mock://test")
            .rate_limit(Some(1)) // 1 req/sec
            .batch_size(3)
            .batch_timeout_ms(100);
        let rpc = Client::new_with_transport(config, transport.clone());

        let addr: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();

        let t0 = std::time::Instant::now();
        let _ = rpc.get_remote_account_info(addr, 1).unwrap();
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() < 500,
            "batch should count as a single request: {elapsed:?}"
        );
        assert_eq!(transport.call_count("mock://test", &batch), 1);
    }

    #[test]
    fn cache_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let transport = MockTransport::default();

        let payload = RpcRequest::GetBlockByNumber { block: 1234 }.to_json_payload(1);
        transport.mock_response(
            "mock://test",
            &payload,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "number": "0x4d2",
                    "timestamp": "0x1",
                    "miner": "0x0000000000000000000000000000000000000000",
                    "gasLimit": "0xffffffffffffffff",
                    "baseFeePerGas": "0x0",
                    "difficulty": "0x0",
                    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000"
                }
            }),
        );

        let config = Config::new("mock://test")
            .cache_dir(tmp.path())
            .batch_size(1);
        let rpc = Client::new_with_transport(config, transport);

        let block = rpc.get_remote_block_info(1234).unwrap();
        assert_eq!(block.number, 1234);

        // Correct path: decimal block number
        let expected = tmp.path().join("eth_getBlockByNumber").join("1234.json");
        assert!(
            expected.exists(),
            "expected cache file at {expected:?}, but it was not found"
        );
    }

    #[test]
    fn get_remote_account_info_parses_batch() {
        let transport = MockTransport::default();
        let addr = json!("0x0000000000000000000000000000000000000000");
        let block_tag = json!("0x1");
        let batch = json!([
            {"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":[addr, block_tag]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getTransactionCount","params":[addr, block_tag]},
            {"jsonrpc":"2.0","id":3,"method":"eth_getCode","params":[addr, block_tag]},
        ]);
        transport.mock_response(
            "mock://test",
            &batch,
            json!([
                {"jsonrpc":"2.0","id":1,"result":"0x4ec7cefe1a0664fd"},
                {"jsonrpc":"2.0","id":2,"result":"0x1707"},
                {"jsonrpc":"2.0","id":3,"result":"0x600160005260016000f3"},
            ]),
        );

        let config = Config::new("mock://test")
            .batch_size(3)
            .batch_timeout_ms(100);
        let rpc = Client::new_with_transport(config, transport.clone());

        let addr = Address::ZERO;
        let (balance, nonce, code) = rpc
            .get_remote_account_info(addr, 1)
            .map(|a| (a.balance, a.nonce, a.code))
            .unwrap();

        assert_eq!(
            balance,
            U256::from_str_radix("4ec7cefe1a0664fd", 16).unwrap()
        );
        assert_eq!(nonce, 0x1707);
        assert!(!code.is_empty());
        assert_eq!(transport.call_count("mock://test", &batch), 1);
    }

    #[test]
    fn get_remote_storage_info_parses_response() {
        let transport = MockTransport::default();
        let payload = RpcRequest::GetStorageAt {
            address: Address::ZERO,
            slot: U256::ZERO,
            block: 1,
        }
        .to_json_payload(1);
        transport.mock_response(
            "mock://test",
            &payload,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": "0x577261707065642045746865720000000000000000000000000000000000001a"
            }),
        );

        let config = Config::new("mock://test").batch_size(1);
        let rpc = Client::new_with_transport(config, transport);

        let slot = rpc
            .get_remote_storage_info(Address::ZERO, U256::ZERO, 1)
            .unwrap();
        assert_eq!(
            slot,
            U256::from_str_radix(
                "577261707065642045746865720000000000000000000000000000000000001a",
                16
            )
            .unwrap()
        );
    }

    fn live_client() -> &'static Client {
        static LIVE_CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
        LIVE_CLIENT.get_or_init(|| {
            let url = std::env::var("RAPTOR_RPC_URL")
                .expect("RAPTOR_RPC_URL must be set to run live tests");
            let config = Config::new(url).batch_size(1);
            Client::new(config)
        })
    }

    #[test]
    fn live_eth_chain_id_returns_positive() {
        let client = live_client();
        let info = client.get_remote_chain_info(1).unwrap();
        assert!(info.chain_id > 0);
    }

    #[test]
    fn live_eth_get_block_by_number_returns_block() {
        let client = live_client();
        let latest = client.get_remote_chain_info(1).unwrap().block.number;
        let block = client.get_remote_block_info(latest).unwrap();
        assert_eq!(block.number, latest);
    }

    #[test]
    fn live_eth_get_balance_returns_balance() {
        let client = live_client();
        let latest = client.get_remote_chain_info(1).unwrap().block.number;
        let vitalik: Address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let info = client.get_remote_account_info(vitalik, latest).unwrap();
        assert!(info.balance > U256::ZERO);
    }

    #[test]
    fn live_eth_get_transaction_count_returns_nonce() {
        let client = live_client();
        let latest = client.get_remote_chain_info(1).unwrap().block.number;
        let vitalik: Address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let info = client.get_remote_account_info(vitalik, latest).unwrap();
        assert!(info.nonce > 0);
    }

    #[test]
    fn live_eth_get_code_returns_contract_bytecode() {
        let client = live_client();
        let latest = client.get_remote_chain_info(1).unwrap().block.number;
        let weth: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();
        let info = client.get_remote_account_info(weth, latest).unwrap();
        assert!(!info.code.is_empty());
    }

    #[test]
    fn live_eth_get_storage_at_returns_slot() {
        let client = live_client();
        let latest = client.get_remote_chain_info(1).unwrap().block.number;
        let weth: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();
        let slot = client
            .get_remote_storage_info(weth, U256::ZERO, latest)
            .unwrap();
        // We only assert the call succeeds; the exact value is not stable.
        let _ = slot;
    }

    #[test]
    fn live_eth_get_account_returns_account_info() {
        let client = live_client();
        let latest = client.get_remote_chain_info(1).unwrap().block.number;
        let weth: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();

        let info = client.get_remote_account_info(weth, latest).unwrap();

        assert!(info.balance > U256::ZERO);
        assert_eq!(info.nonce, 1); // WETH deploy nonce
        assert!(!info.code.is_empty());
    }
}
