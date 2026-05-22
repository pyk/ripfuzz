//! JSON-RPC client with caching, deduplication, rate limiting, and retries.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Address, B256, Bytes, U64, U256};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use tracing::{instrument, trace};
use ureq::Agent;

use crate::rpc_v2::config::Config;
use crate::rpc_v2::request;
use crate::rpc_v2::transport::Transport;
use crate::rpc_v2::{Cache, DedupTable, RateLimiter};

/// Typed block header returned by `eth_getBlockByNumber`.
#[derive(Debug, Clone, Deserialize)]
pub struct Block {
    #[serde(rename = "number")]
    pub number: U64,
    #[serde(rename = "timestamp")]
    pub timestamp: U64,
    #[serde(rename = "miner")]
    pub coinbase: Address,
    #[serde(rename = "gasLimit")]
    pub gas_limit: U64,
    #[serde(rename = "baseFeePerGas", default)]
    pub basefee: U64,
    #[serde(rename = "mixHash", default)]
    pub prevrandao: Option<B256>,
    #[serde(rename = "difficulty", default)]
    pub difficulty: U256,
    #[serde(rename = "excessBlobGas", default)]
    pub excess_blob_gas: Option<U64>,
    #[serde(rename = "hash", default)]
    pub hash: Option<B256>,
}

/// JSON-RPC client with two-layer caching, deduplication, rate limiting,
/// and retries.
#[derive(Clone, Debug)]
pub struct Client {
    inner: Arc<ClientInner>,
}

#[derive(Debug)]
struct ClientInner {
    transport: Box<dyn Transport>,
    url: String,
    retries: u32,
    backoff: Duration,
    cache: Option<Cache>,
    dedup: DedupTable,
    limiter: Option<RateLimiter>,
    chain_id: u64,
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
        let cache = config.cache_dir.map(|dir| Cache::new(dir, config.chain_id));
        let url = config.url.unwrap_or_default();
        Self {
            inner: Arc::new(ClientInner {
                transport: Box::new(transport),
                url,
                retries: config.retries,
                backoff: Duration::from_millis(config.backoff_ms),
                cache,
                dedup: DedupTable::new(),
                limiter,
                chain_id: config.chain_id,
            }),
        }
    }

    /// Compute the sleep duration for a given retry attempt (0-indexed).
    fn backoff_duration(&self, attempt: u32) -> Duration {
        let multiplier = 2_u64.pow(attempt);
        self.inner.backoff * multiplier as u32
    }

    /// Configured chain ID.
    pub fn chain_id(&self) -> u64 {
        self.inner.chain_id
    }

    /// Query the remote node for its latest block number.
    pub fn latest_block_number(&self) -> Result<u64> {
        let payload = request::payload("eth_blockNumber", &[]);
        let envelope = self.call("latest_block_number", payload)?;
        let result = envelope
            .get("result")
            .cloned()
            .with_context(|| "missing result field in eth_blockNumber response")?;
        let number = result
            .as_str()
            .context("missing block number in response")?;
        let hex = number.strip_prefix("0x").unwrap_or(number);
        u64::from_str_radix(hex, 16).context("invalid block number")
    }

    /// Fetch a block header by number.
    pub fn get_block_by_number(&self, block: u64) -> Result<Block> {
        let cache_key = format!("get_block_by_number_{block}");
        let payload = request::payload(
            "eth_getBlockByNumber",
            &[json!(format!("0x{block:x}")), json!(false)],
        );
        let envelope = self.call(&cache_key, payload)?;
        let result = envelope
            .get("result")
            .cloned()
            .with_context(|| "missing result field in eth_getBlockByNumber response")?;
        if result.is_null() {
            bail!("block {} not found", block);
        }
        serde_json::from_value(result).context("invalid block response")
    }

    /// Fetch an account balance at a specific block.
    pub fn get_balance(&self, address: Address, block: u64) -> Result<U256> {
        let cache_key = format!("get_balance_{block}_{address:x}");
        let payload = request::payload(
            "eth_getBalance",
            &[
                json!(format!("0x{address:x}")),
                json!(format!("0x{block:x}")),
            ],
        );
        let envelope = self.call(&cache_key, payload)?;
        let result = envelope
            .get("result")
            .cloned()
            .with_context(|| "missing result field in eth_getBalance response")?;
        let s = result.as_str().context("expected hex string")?;
        s.parse().context("invalid u256 hex")
    }

    /// Fetch an account nonce at a specific block.
    pub fn get_transaction_count(&self, address: Address, block: u64) -> Result<u64> {
        let cache_key = format!("get_transaction_count_{block}_{address:x}");
        let payload = request::payload(
            "eth_getTransactionCount",
            &[
                json!(format!("0x{address:x}")),
                json!(format!("0x{block:x}")),
            ],
        );
        let envelope = self.call(&cache_key, payload)?;
        let result = envelope
            .get("result")
            .cloned()
            .with_context(|| "missing result field in eth_getTransactionCount response")?;
        let s = result.as_str().context("expected hex string")?;
        let u: U64 = s.parse().context("invalid u64 hex")?;
        Ok(u.to())
    }

    /// Fetch contract bytecode at a specific block.
    pub fn get_code(&self, address: Address, block: u64) -> Result<Bytes> {
        let cache_key = format!("get_code_{block}_{address:x}");
        let payload = request::payload(
            "eth_getCode",
            &[
                json!(format!("0x{address:x}")),
                json!(format!("0x{block:x}")),
            ],
        );
        let envelope = self.call(&cache_key, payload)?;
        let result = envelope
            .get("result")
            .cloned()
            .with_context(|| "missing result field in eth_getCode response")?;
        let s = result.as_str().context("expected hex string")?;
        s.parse().context("invalid hex bytes")
    }

    /// Fetch a storage slot at a specific block.
    pub fn get_storage_at(&self, address: Address, slot: U256, block: u64) -> Result<U256> {
        let cache_key = format!("get_storage_at_{block}_{slot:x}_{address:x}");
        let payload = request::payload(
            "eth_getStorageAt",
            &[
                json!(format!("0x{address:x}")),
                json!(format!("0x{slot:x}")),
                json!(format!("0x{block:x}")),
            ],
        );
        let envelope = self.call(&cache_key, payload)?;
        let result = envelope
            .get("result")
            .cloned()
            .with_context(|| "missing result field in eth_getStorageAt response")?;
        let s = result.as_str().context("expected hex string")?;
        s.parse().context("invalid u256 hex")
    }

    /// Fetch balance, nonce, and code for an address in a single batch request.
    ///
    /// Responses are matched by their JSON-RPC `id`, not by array position,
    /// and any RPC error inside the batch aborts the whole call.
    pub fn get_account(&self, address: Address, block: u64) -> Result<(U256, u64, Bytes)> {
        let addr = json!(format!("0x{address:x}"));
        let block_tag = json!(format!("0x{block:x}"));
        let cache_key = format!("get_account_{block}_{address:x}");

        let id_balance: u64 = 100;
        let id_nonce: u64 = 101;
        let id_code: u64 = 102;

        let payload = serde_json::Value::Array(vec![
            request::payload_with_id(
                "eth_getBalance",
                &[addr.clone(), block_tag.clone()],
                id_balance,
            ),
            request::payload_with_id(
                "eth_getTransactionCount",
                &[addr.clone(), block_tag.clone()],
                id_nonce,
            ),
            request::payload_with_id("eth_getCode", &[addr, block_tag], id_code),
        ]);

        let envelope = self.call(&cache_key, payload)?;
        let results = envelope
            .as_array()
            .context("batch response should be an array")?;

        let mut by_id = HashMap::new();
        for item in results {
            if let Some(id) = item.get("id").and_then(|v| v.as_u64()) {
                by_id.insert(id, item);
            }
        }

        let balance_item = by_id
            .get(&id_balance)
            .with_context(|| "missing eth_getBalance response in batch")?;
        let balance: U256 = balance_item
            .get("result")
            .and_then(|v| v.as_str())
            .context("expected hex string for balance")?
            .parse()
            .context("invalid u256 hex for balance")?;

        let nonce_item = by_id
            .get(&id_nonce)
            .with_context(|| "missing eth_getTransactionCount response in batch")?;
        let nonce_str = nonce_item
            .get("result")
            .and_then(|v| v.as_str())
            .context("expected hex string for nonce")?;
        let nonce: u64 = U64::from_str_radix(nonce_str.strip_prefix("0x").unwrap_or(nonce_str), 16)
            .context("invalid u64 hex for nonce")?
            .to();

        let code_item = by_id
            .get(&id_code)
            .with_context(|| "missing eth_getCode response in batch")?;
        let code: Bytes = code_item
            .get("result")
            .and_then(|v| v.as_str())
            .context("expected hex string for code")?
            .parse()
            .context("invalid hex bytes for code")?;

        Ok((balance, nonce, code))
    }

    // -----------------------------------------------------------------
    // Internal call pipeline
    // -----------------------------------------------------------------

    /// Unified internal call that handles deduplication, caching, rate
    /// limiting, retries, and transport for both single requests and batches.
    ///
    /// The caller builds the JSON-RPC `request_payload` (a single object or an
    /// array for batching) and is responsible for extracting `result` fields
    /// from the returned envelope.
    #[instrument(skip(self, request_payload), fields(cache_key))]
    fn call(
        &self,
        cache_key: &str,
        request_payload: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // 1. Deduplication
        if let Some(result) = self.inner.dedup.register(cache_key) {
            trace!(%cache_key, "dedup hit");
            return result;
        }
        let guard = self.inner.dedup.guard(cache_key);

        // 2. Memory cache
        let skip_cache = cache_key == "latest_block_number";
        if !skip_cache
            && let Some(ref cache) = self.inner.cache
            && let Some(envelope) = cache.get(cache_key)
        {
            trace!(%cache_key, "cache hit");
            self.inner.dedup.complete(cache_key, Ok(envelope.clone()));
            guard.deactivate();
            return Ok(envelope);
        }

        // 3. Rate limit gate
        // A batch is dispatched as a single HTTP POST, so it consumes
        // exactly one token regardless of how many JSON-RPC methods it
        // contains (issue #2).
        if let Some(ref limiter) = self.inner.limiter {
            trace!(%cache_key, "rate limit acquire");
            limiter.acquire();
        }

        // 4. Live network fetch with retries
        let value = 'retry: {
            let mut last_err: Option<anyhow::Error> = None;
            for attempt in 0..=self.inner.retries {
                match self.inner.transport.exec(&self.inner.url, &request_payload) {
                    Ok(value) => {
                        // Validate response shape and reject RPC errors.
                        let rpc_error = if let Some(arr) = value.as_array() {
                            arr.iter()
                                .find_map(|item| item.get("error"))
                                .map(|e| format!("RPC error in batch response: {e}"))
                        } else if let Some(obj) = value.as_object() {
                            obj.get("error").map(|e| format!("RPC error: {e}"))
                        } else {
                            Some("invalid RPC response".into())
                        };

                        if let Some(err_msg) = rpc_error {
                            last_err = Some(anyhow::anyhow!("{err_msg}"));
                            if attempt < self.inner.retries {
                                std::thread::sleep(self.backoff_duration(attempt));
                                continue;
                            }
                            break;
                        }
                        break 'retry value;
                    }
                    Err(e) => {
                        last_err = Some(e);
                        if attempt < self.inner.retries {
                            std::thread::sleep(self.backoff_duration(attempt));
                        }
                    }
                }
            }
            let err = last_err.unwrap_or_else(|| anyhow::anyhow!("RPC request failed"));
            self.inner
                .dedup
                .complete(cache_key, Err(anyhow::anyhow!("{err}")));
            guard.deactivate();
            return Err(err);
        };

        // 5. Update cache and complete dedup
        if !skip_cache && let Some(ref cache) = self.inner.cache {
            cache.insert(cache_key, value.clone());
        }
        self.inner.dedup.complete(cache_key, Ok(value.clone()));
        guard.deactivate();
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    #[test]
    fn backoff_is_exponential() {
        let client = Client::new_with_transport(
            Config::new().url("mock://test").chain_id(1).backoff_ms(100),
            crate::rpc_v2::transport::MockTransport::default(),
        );
        assert_eq!(client.backoff_duration(0), Duration::from_millis(100));
        assert_eq!(client.backoff_duration(1), Duration::from_millis(200));
        assert_eq!(client.backoff_duration(2), Duration::from_millis(400));
        assert_eq!(client.backoff_duration(3), Duration::from_millis(800));
    }

    #[test]
    fn mock_transport_roundtrip() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        let payload = request::payload("eth_blockNumber", &[]);
        transport.insert(
            "mock://test",
            &payload,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );

        let config = Config::new().url("mock://test").chain_id(1);
        let rpc = Client::new_with_transport(config, transport);

        let result = rpc.latest_block_number().unwrap();
        assert_eq!(result, 0x1a2b);
    }

    #[test]
    fn dedup_coalesces_parallel_requests() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        transport.set_delay(Duration::from_millis(100));
        let payload = request::payload("eth_blockNumber", &[]);
        transport.insert(
            "mock://test",
            &payload,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );

        let config = Config::new().url("mock://test").chain_id(1);
        let rpc = Client::new_with_transport(config, transport.clone());
        let rpc2 = rpc.clone();

        let t1 = std::thread::spawn(move || rpc.latest_block_number());
        let t2 = std::thread::spawn(move || rpc2.latest_block_number());

        let r1 = t1.join().unwrap().unwrap();
        let r2 = t2.join().unwrap().unwrap();
        assert_eq!(r1, r2);
        assert_eq!(r1, 0x1a2b);
        assert_eq!(transport.call_count("mock://test", &payload), 1);
    }

    #[test]
    fn rate_limit_throttles_without_network() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        let payload = request::payload("eth_blockNumber", &[]);
        transport.insert(
            "mock://test",
            &payload,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1"}),
        );

        let config = Config::new()
            .url("mock://test")
            .chain_id(1)
            .rate_limit(Some(2));
        let rpc = Client::new_with_transport(config, transport);

        let t0 = std::time::Instant::now();
        for _ in 0..4 {
            let _ = rpc.latest_block_number().unwrap();
        }
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() >= 800,
            "rate limit did not throttle: {elapsed:?}"
        );
    }

    /// Regression (issue #2): a JSON-RPC batch is dispatched as a single HTTP
    /// POST, so it must consume exactly one rate-limit token.
    #[test]
    fn rate_limit_batch_counts_as_single_request() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        let addr = json!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        let block_tag = json!("0x1");
        let batch = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_getBalance","params":[addr, block_tag]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getTransactionCount","params":[addr, block_tag]},
            {"jsonrpc":"2.0","id":102,"method":"eth_getCode","params":[addr, block_tag]},
        ]);
        transport.insert(
            "mock://test",
            &batch,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x4ec7cefe1a0664fd"},
                {"jsonrpc":"2.0","id":101,"result":"0x1707"},
                {"jsonrpc":"2.0","id":102,"result":"0x6060604052"},
            ]),
        );

        let config = Config::new()
            .url("mock://test")
            .chain_id(1)
            .rate_limit(Some(1)); // 1 req/sec
        let rpc = Client::new_with_transport(config, transport.clone());

        let addr: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();

        let t0 = std::time::Instant::now();
        let _ = rpc.get_account(addr, 1).unwrap();
        let elapsed = t0.elapsed();

        assert!(
            elapsed.as_millis() < 200,
            "batch over-counted for rate limit, elapsed: {elapsed:?}"
        );
    }

    /// Stress-test deduplication with many threads hitting the same request.
    #[test]
    fn dedup_coalesces_many_parallel_requests() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        transport.set_delay(Duration::from_millis(100));
        let payload = request::payload("eth_blockNumber", &[]);
        transport.insert(
            "mock://test",
            &payload,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );

        let config = Config::new().url("mock://test").chain_id(1);
        let rpc = Client::new_with_transport(config, transport.clone());

        let mut handles = Vec::new();
        for _ in 0..10 {
            let rpc_clone = rpc.clone();
            handles.push(std::thread::spawn(move || rpc_clone.latest_block_number()));
        }

        let results: Vec<u64> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();

        assert!(results.iter().all(|&r| r == 0x1a2b));
        assert_eq!(transport.call_count("mock://test", &payload), 1);
    }

    /// Use a barrier to release all threads at the exact same instant,
    /// maximizing the race window and proving the dedup table is sound.
    #[test]
    fn dedup_with_barrier_maximizes_contention() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        transport.set_delay(Duration::from_millis(200));
        let payload = request::payload("eth_blockNumber", &[]);
        transport.insert(
            "mock://test",
            &payload,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0xdeadbeef"}),
        );

        let config = Config::new().url("mock://test").chain_id(1);
        let rpc = Client::new_with_transport(config, transport.clone());

        let thread_count = 8;
        let barrier = Arc::new(std::sync::Barrier::new(thread_count));
        let mut handles = Vec::with_capacity(thread_count);

        for _ in 0..thread_count {
            let rpc_clone = rpc.clone();
            let barrier_clone = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier_clone.wait();
                rpc_clone.latest_block_number()
            }));
        }

        let results: Vec<u64> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();

        assert!(results.iter().all(|&r| r == 0xdeadbeef));
        assert_eq!(transport.call_count("mock://test", &payload), 1);
    }

    /// Verify that deduplication is keyed by (method, params).
    /// Two distinct requests issued from parallel threads must each
    /// be dispatched exactly once.
    #[test]
    fn dedup_only_coalesces_identical_requests() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        transport.set_delay(Duration::from_millis(50));
        let payload_a = request::payload(
            "eth_getBalance",
            &[
                json!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                json!("0x112233"),
            ],
        );
        let payload_b = request::payload(
            "eth_getBalance",
            &[
                json!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                json!("0x112233"),
            ],
        );
        transport.insert(
            "mock://test",
            &payload_a,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1"}),
        );
        transport.insert(
            "mock://test",
            &payload_b,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x2"}),
        );

        let config = Config::new().url("mock://test").chain_id(1);
        let rpc = Client::new_with_transport(config, transport.clone());

        let addr_a: Address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .parse()
            .unwrap();
        let addr_b: Address = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .parse()
            .unwrap();

        let mut handles = Vec::new();
        for _ in 0..5 {
            let rpc_clone = rpc.clone();
            handles.push(std::thread::spawn(move || {
                rpc_clone.get_balance(addr_a, 0x112233)
            }));
        }
        for _ in 0..5 {
            let rpc_clone = rpc.clone();
            handles.push(std::thread::spawn(move || {
                rpc_clone.get_balance(addr_b, 0x112233)
            }));
        }

        let results: Vec<U256> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();

        for r in &results[..5] {
            assert_eq!(*r, U256::from(1));
        }
        for r in &results[5..] {
            assert_eq!(*r, U256::from(2));
        }

        assert_eq!(transport.call_count("mock://test", &payload_a), 1);
        assert_eq!(transport.call_count("mock://test", &payload_b), 1);
    }

    /// Rate limit + dedup interaction: the first batch consumes the initial
    /// token bucket. A second parallel batch for the *same* request must still
    /// wait for the rate-limit refill before the leader can dispatch.
    #[test]
    fn rate_limit_throttles_parallel_deduped_requests() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        transport.set_delay(Duration::from_millis(50));
        let payload = request::payload("eth_blockNumber", &[]);
        transport.insert(
            "mock://test",
            &payload,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1"}),
        );

        let config = Config::new()
            .url("mock://test")
            .chain_id(1)
            .rate_limit(Some(1)); // 1 req/sec; no cache_dir => no cache layer
        let rpc = Client::new_with_transport(config, transport.clone());

        // Batch 1 – token available, should complete immediately.
        let mut handles = Vec::new();
        let t0 = std::time::Instant::now();
        for _ in 0..5 {
            let rpc_clone = rpc.clone();
            handles.push(std::thread::spawn(move || rpc_clone.latest_block_number()));
        }
        let results1: Vec<u64> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();
        let elapsed1 = t0.elapsed();

        assert!(results1.iter().all(|&r| r == 1));
        assert!(
            elapsed1.as_millis() < 200,
            "first batch should not be throttled: {elapsed1:?}"
        );

        // Batch 2 – token exhausted. Leader must wait ~1000 ms for refill.
        let mut handles = Vec::new();
        let t0 = std::time::Instant::now();
        for _ in 0..5 {
            let rpc_clone = rpc.clone();
            handles.push(std::thread::spawn(move || rpc_clone.latest_block_number()));
        }
        let results2: Vec<u64> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();
        let elapsed2 = t0.elapsed();

        assert!(results2.iter().all(|&r| r == 1));
        assert!(
            elapsed2.as_millis() >= 800,
            "second batch should be rate-limited: {elapsed2:?}"
        );
        assert_eq!(transport.call_count("mock://test", &payload), 2);
    }

    /// Maximal contention: a barrier releases many threads at the exact same
    /// instant *after* the token bucket is empty. Only one dispatch happens,
    /// but that dispatch is delayed by the rate limiter.
    #[test]
    fn rate_limit_with_barrier_and_dedup() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        transport.set_delay(Duration::from_millis(50));
        let payload = request::payload("eth_blockNumber", &[]);
        transport.insert(
            "mock://test",
            &payload,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0xcafe"}),
        );

        let config = Config::new()
            .url("mock://test")
            .chain_id(1)
            .rate_limit(Some(1));
        let rpc = Client::new_with_transport(config, transport.clone());

        // Burn the single initial token.
        let _ = rpc.latest_block_number().unwrap();

        let thread_count = 20;
        let barrier = Arc::new(std::sync::Barrier::new(thread_count));
        let mut handles = Vec::with_capacity(thread_count);

        let t0 = std::time::Instant::now();
        for _ in 0..thread_count {
            let rpc_clone = rpc.clone();
            let barrier_clone = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier_clone.wait();
                rpc_clone.latest_block_number()
            }));
        }

        let results: Vec<u64> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();
        let elapsed = t0.elapsed();

        assert!(results.iter().all(|&r| r == 0xcafe));
        assert!(
            elapsed.as_millis() >= 800,
            "rate limit did not throttle under contention: {elapsed:?}"
        );
        assert_eq!(transport.call_count("mock://test", &payload), 2);
    }

    /// Verify that `eth_blockNumber` bypasses the cache while other
    /// methods are still cached. Sequential calls to `latest_block_number`
    /// must hit the transport every time, whereas `get_balance` should be
    /// served from cache on the second call.
    #[test]
    fn eth_block_number_skips_cache_but_other_methods_are_cached() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        let payload_bn = request::payload("eth_blockNumber", &[]);
        let payload_bal = request::payload(
            "eth_getBalance",
            &[
                json!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                json!("0x1"),
            ],
        );
        transport.insert(
            "mock://test",
            &payload_bn,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );
        transport.insert(
            "mock://test",
            &payload_bal,
            json!({"jsonrpc": "2.0", "id": 1, "result": "0xc0ffee"}),
        );

        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new()
            .url("mock://test")
            .chain_id(1)
            .cache_dir(tmp.path());
        let rpc = Client::new_with_transport(config, transport.clone());

        let addr: Address = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .parse()
            .unwrap();

        // First call to each method hits the transport.
        assert_eq!(rpc.latest_block_number().unwrap(), 0x1a2b);
        assert_eq!(rpc.get_balance(addr, 1).unwrap(), U256::from(0xc0ffee));

        // Second call: eth_blockNumber should hit the transport again (not
        // cached), while eth_getBalance should be served from cache.
        assert_eq!(rpc.latest_block_number().unwrap(), 0x1a2b);
        assert_eq!(rpc.get_balance(addr, 1).unwrap(), U256::from(0xc0ffee));

        assert_eq!(transport.call_count("mock://test", &payload_bn), 2);
        assert_eq!(transport.call_count("mock://test", &payload_bal), 1);
    }

    #[test]
    fn mock_get_account_roundtrip() {
        let transport = crate::rpc_v2::transport::MockTransport::default();
        let addr = json!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        let block_tag = json!("0x17fa30b");
        let batch = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_getBalance","params":[addr, block_tag]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getTransactionCount","params":[addr, block_tag]},
            {"jsonrpc":"2.0","id":102,"method":"eth_getCode","params":[addr, block_tag]},
        ]);
        transport.insert(
            "mock://test",
            &batch,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x4ec7cefe1a0664fd"},
                {"jsonrpc":"2.0","id":101,"result":"0x1707"},
                {"jsonrpc":"2.0","id":102,"result":"0x6060604052"},
            ]),
        );

        let config = Config::new().url("mock://test").chain_id(1);
        let rpc = Client::new_with_transport(config, transport.clone());

        let weth: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();
        let (balance, nonce, code) = rpc.get_account(weth, 0x17fa30b).unwrap();

        assert_eq!(
            balance,
            U256::from_str_radix("4ec7cefe1a0664fd", 16).unwrap()
        );
        assert_eq!(nonce, 0x1707);
        assert!(!code.is_empty());
        // get_account issues a single JSON-RPC batch; the mock must reflect
        // one batch dispatch rather than three individual sends.
        assert_eq!(transport.call_count("mock://test", &batch), 1);
    }

    // -----------------------------------------------------------------
    // Issue #1 reproduction: batch response ordering & error caching
    // -----------------------------------------------------------------

    /// Transport that returns batch responses in *reverse* order,
    /// echoing back the request `id` like a real JSON-RPC node.
    #[derive(Debug, Clone)]
    struct ReversedBatchTransport;

    impl crate::rpc_v2::transport::Transport for ReversedBatchTransport {
        fn exec(&self, _url: &str, payload: &serde_json::Value) -> Result<serde_json::Value> {
            let items = payload.as_array().context("expected batch")?;
            let mut responses = Vec::with_capacity(items.len());
            for payload in items {
                let method = payload.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let id = payload.get("id").cloned().unwrap_or(json!(0));
                let resp = match method {
                    "eth_getBalance" => {
                        json!({"jsonrpc":"2.0","id":id,"result":"0x00000000000000000000000000000000000000000000000000000000DEADBEEF"})
                    }
                    "eth_getTransactionCount" => {
                        json!({"jsonrpc":"2.0","id":id,"result":"0x2"})
                    }
                    "eth_getCode" => {
                        json!({"jsonrpc":"2.0","id":id,"result":"0xCAFEBABE"})
                    }
                    _ => json!({"jsonrpc":"2.0","id":id,"result":null}),
                };
                responses.push(resp);
            }
            // Reverse the order: JSON-RPC spec does not guarantee ordering.
            responses.reverse();
            Ok(json!(responses))
        }
    }

    // Regression test: batch responses may arrive out-of-order.
    // `get_account` must match by `id`, not by array position.
    #[test]
    fn get_account_matches_responses_by_id() {
        let transport = ReversedBatchTransport;
        let config = Config::new().url("mock://test").chain_id(1);
        let rpc = Client::new_with_transport(config, transport);

        let addr: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();
        let (balance, nonce, code) = rpc.get_account(addr, 1).unwrap();

        assert_eq!(
            balance,
            "0x00000000000000000000000000000000000000000000000000000000DEADBEEF"
                .parse::<U256>()
                .unwrap()
        );
        assert_eq!(nonce, 2);
        assert_eq!(code, "0xCAFEBABE".parse::<Bytes>().unwrap());
    }

    /// Transport that returns a batch containing one error on the first call,
    /// and a clean batch on the second call. Echoes back request `id`s.
    #[derive(Debug, Clone)]
    struct ErrorThenSuccessTransport(Arc<std::sync::atomic::AtomicUsize>);

    impl crate::rpc_v2::transport::Transport for ErrorThenSuccessTransport {
        fn exec(&self, _url: &str, payload: &serde_json::Value) -> Result<serde_json::Value> {
            let count = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let items = payload.as_array().context("expected batch")?;
            let ids: Vec<serde_json::Value> = items
                .iter()
                .map(|p| p.get("id").cloned().unwrap_or(json!(0)))
                .collect();
            if count == 0 {
                // First call: balance item is an RPC error.
                Ok(json!([
                    {"jsonrpc":"2.0","id":ids[0],"error":{"code":-32000,"message":"rate limited"}},
                    {"jsonrpc":"2.0","id":ids[1],"result":"0x1"},
                    {"jsonrpc":"2.0","id":ids[2],"result":"0x6000"},
                ]))
            } else {
                // Second call: clean data.
                Ok(json!([
                    {"jsonrpc":"2.0","id":ids[0],"result":"0xDEADBEEF"},
                    {"jsonrpc":"2.0","id":ids[1],"result":"0x1"},
                    {"jsonrpc":"2.0","id":ids[2],"result":"0x6000"},
                ]))
            }
        }
    }

    // Regression test: a batch containing an RPC error must NOT be cached.
    // The client retries, and the successful retry result is what gets
    // cached. A subsequent call must be served from cache.
    #[test]
    fn get_account_batch_error_retries_and_caches() {
        let transport = ErrorThenSuccessTransport(Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new()
            .url("mock://test")
            .chain_id(1)
            .cache_dir(tmp.path());
        let rpc = Client::new_with_transport(config, transport.clone());

        let addr: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();

        // First call encounters a batch RPC error, retries, and succeeds.
        let (balance, nonce, code) = rpc.get_account(addr, 1).unwrap();
        assert_eq!(balance, U256::from(0xDEADBEEFu64));
        assert_eq!(nonce, 1);
        assert_eq!(code, "0x6000".parse::<Bytes>().unwrap());

        // The transport was called twice (error, then retry).
        assert_eq!(transport.0.load(std::sync::atomic::Ordering::SeqCst), 2);

        // Second call must be served from cache.
        let (balance2, nonce2, code2) = rpc.get_account(addr, 1).unwrap();
        assert_eq!(balance2, U256::from(0xDEADBEEFu64));
        assert_eq!(nonce2, 1);
        assert_eq!(code2, "0x6000".parse::<Bytes>().unwrap());

        // No additional transport calls.
        assert_eq!(transport.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// Regression (issue #4): Block must reject JSON missing critical fields.
    #[test]
    fn block_deserialize_missing_required_fields() {
        let cases: Vec<(&str, &str)> = vec![
            (
                "missing number",
                r#"{"timestamp":"0x65f066f8","gasLimit":"0x1c9c380","miner":"0x1234567890123456789012345678901234567890"}"#,
            ),
            (
                "missing timestamp",
                r#"{"number":"0x1","gasLimit":"0x1c9c380","miner":"0x1234567890123456789012345678901234567890"}"#,
            ),
            (
                "missing gasLimit",
                r#"{"number":"0x1","timestamp":"0x65f066f8","miner":"0x1234567890123456789012345678901234567890"}"#,
            ),
            (
                "missing miner (coinbase)",
                r#"{"number":"0x1","timestamp":"0x65f066f8","gasLimit":"0x1c9c380"}"#,
            ),
        ];

        for (label, json) in cases {
            let result: Result<Block, serde_json::Error> = serde_json::from_str(json);
            assert!(
                result.is_err(),
                "Expected deserialization to fail for {label}, but got: {result:?}"
            );
        }
    }

    /// Regression: cache files must be stored under `rpc/{chain_id}/` and
    /// block numbers in the filename must be decimal, not hex.
    #[test]
    fn cache_path_uses_decimal_block_and_chain_id() {
        let tmp = tempfile::tempdir().unwrap();
        let transport = crate::rpc_v2::transport::MockTransport::default();
        let payload = request::payload("eth_getBlockByNumber", &[json!("0x4d2"), json!(false)]);
        transport.insert(
            "mock://test",
            &payload,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "number": "0x4d2",
                    "timestamp": "0x0",
                    "miner": "0x0000000000000000000000000000000000000000",
                    "gasLimit": "0x0",
                    "baseFeePerGas": "0x0"
                }
            }),
        );

        let config = Config::new()
            .url("mock://test")
            .chain_id(1)
            .cache_dir(tmp.path());
        let rpc = Client::new_with_transport(config, transport);

        let block = rpc.get_block_by_number(1234).unwrap();
        assert_eq!(block.number.to::<u64>(), 1234);

        // Correct path: chain_id directory + decimal block number
        let expected = tmp
            .path()
            .join("rpc")
            .join("1")
            .join("get_block_by_number_1234.json");
        assert!(
            expected.exists(),
            "expected cache file at {expected:?}, but it was not found"
        );

        // Old buggy path must not exist
        let buggy = tmp.path().join("rpc").join("get_block_by_number_4d2.json");
        assert!(!buggy.exists(), "unexpected buggy cache file at {buggy:?}");
    }

    fn live_client() -> &'static Client {
        static LIVE_CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
        LIVE_CLIENT.get_or_init(|| {
            let url = std::env::var("RAPTOR_RPC_URL")
                .expect("RAPTOR_RPC_URL must be set to run live tests");
            let config = Config::new().url(url).chain_id(1);
            Client::new(config)
        })
    }

    #[test]
    fn live_eth_block_number_returns_positive() {
        let client = live_client();
        let block = client.latest_block_number().unwrap();
        assert!(block > 0);
    }

    #[test]
    fn live_eth_get_block_by_number_returns_block() {
        let client = live_client();
        let latest = client.latest_block_number().unwrap();
        let block = client.get_block_by_number(latest).unwrap();
        assert_eq!(block.number.to::<u64>(), latest);
    }

    #[test]
    fn live_eth_get_balance_returns_balance() {
        let client = live_client();
        let latest = client.latest_block_number().unwrap();
        let vitalik: Address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let balance = client.get_balance(vitalik, latest).unwrap();
        assert!(balance > U256::ZERO);
    }

    #[test]
    fn live_eth_get_transaction_count_returns_nonce() {
        let client = live_client();
        let latest = client.latest_block_number().unwrap();
        let vitalik: Address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let nonce = client.get_transaction_count(vitalik, latest).unwrap();
        assert!(nonce > 0);
    }

    #[test]
    fn live_eth_get_code_returns_contract_bytecode() {
        let client = live_client();
        let latest = client.latest_block_number().unwrap();
        let weth: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();
        let code = client.get_code(weth, latest).unwrap();
        assert!(!code.is_empty());
    }

    #[test]
    fn live_eth_get_storage_at_returns_slot() {
        let client = live_client();
        let latest = client.latest_block_number().unwrap();
        let weth: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();
        let slot = client.get_storage_at(weth, U256::ZERO, latest).unwrap();
        // We only assert the call succeeds; the exact value is not stable.
        let _ = slot;
    }

    #[test]
    fn live_eth_get_account_returns_account_info() {
        let client = live_client();
        let latest = client.latest_block_number().unwrap();
        let weth: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();

        let (balance, nonce, code) = client.get_account(weth, latest).unwrap();

        assert!(balance > U256::ZERO);
        assert_eq!(nonce, 1); // WETH deploy nonce
        assert!(!code.is_empty());
    }
}
