//! HTTP agent pool, URL pool, and JSON-RPC client with caching, deduplication,
//! rate limiting, retries, and failover.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use alloy_primitives::{Address, B256, Bytes, U64, U256};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;
use tracing::{instrument, trace};
use ureq::Agent;

use crate::rpc_v2::config::Config;
use crate::rpc_v2::request;
use crate::rpc_v2::transport::{HttpTransport, Transport};
use crate::rpc_v2::{Cache, DedupTable, RateLimiter};

/// Round-robin pool of `ureq::Agent` instances.
#[derive(Debug)]
pub struct AgentPool {
    agents: Vec<Agent>,
    idx: AtomicUsize,
}

impl AgentPool {
    pub fn new(agents: Vec<Agent>) -> Self {
        Self {
            agents,
            idx: AtomicUsize::new(0),
        }
    }

    pub fn next(&self) -> &Agent {
        let idx = self.idx.fetch_add(1, Ordering::Relaxed) % self.agents.len().max(1);
        &self.agents[idx]
    }
}

/// Round-robin pool of JSON-RPC endpoint URLs.
#[derive(Debug)]
pub struct UrlPool {
    urls: Vec<String>,
    idx: AtomicUsize,
}

impl UrlPool {
    pub fn new(urls: Vec<String>) -> Self {
        Self {
            urls,
            idx: AtomicUsize::new(0),
        }
    }

    pub fn next(&self) -> &str {
        let idx = self.idx.fetch_add(1, Ordering::Relaxed) % self.urls.len().max(1);
        &self.urls[idx]
    }

    pub fn urls(&self) -> &[String] {
        &self.urls
    }
}

/// Typed block header returned by `eth_getBlockByNumber`.
#[derive(Debug, Clone, Deserialize)]
pub struct Block {
    #[serde(rename = "number", default)]
    pub number: U64,
    #[serde(rename = "timestamp", default)]
    pub timestamp: U64,
    #[serde(rename = "miner", default)]
    pub coinbase: Address,
    #[serde(rename = "gasLimit", default = "default_gas_limit")]
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

fn default_gas_limit() -> U64 {
    U64::from(30_000_000)
}

/// JSON-RPC client with two-layer caching, deduplication, rate limiting,
/// retries, and failover across multiple URLs.
#[derive(Clone, Debug)]
pub struct Client {
    inner: Arc<ClientInner>,
}

#[derive(Debug)]
struct ClientInner {
    transport: Arc<dyn Transport>,
    cache: Option<Cache>,
    dedup: DedupTable,
    limiter: Option<RateLimiter>,
    chain_id: u64,
}

impl Client {
    /// Create a new client from a validated configuration.
    pub fn new(config: Config) -> Self {
        let timeout = Duration::from_millis(config.timeout_ms);
        let pool_size = config.pool_size.max(1) as usize;
        let agents: Vec<Agent> = (0..pool_size)
            .map(|_| {
                let cfg = Agent::config_builder()
                    .timeout_global(Some(timeout))
                    .build();
                Agent::new_with_config(cfg)
            })
            .collect();
        let transport = Arc::new(HttpTransport::new(
            config.urls.clone(),
            agents,
            config.retries,
            Duration::from_millis(config.backoff_ms),
        ));
        Self::new_with_transport(config, transport)
    }

    /// Create a new client with a custom transport (e.g. for testing).
    pub fn new_with_transport(config: Config, transport: Arc<dyn Transport>) -> Self {
        let limiter = config.rate_limit.map(RateLimiter::new);
        let cache = config.cache_dir.map(Cache::new);
        Self {
            inner: Arc::new(ClientInner {
                transport,
                cache,
                dedup: DedupTable::new(),
                limiter,
                chain_id: config.chain_id,
            }),
        }
    }

    /// Cache key derived from the configured chain ID.
    pub fn cache_key(&self) -> String {
        format!("{}", self.inner.chain_id)
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
        let cache_key = format!("get_block_by_number_{block:x}");
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
        let cache_key = format!("get_balance_{block:x}_{address:x}");
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
        let cache_key = format!("get_transaction_count_{block:x}_{address:x}");
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
        let cache_key = format!("get_code_{block:x}_{address:x}");
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
        let cache_key = format!("get_storage_at_{block:x}_{slot:x}_{address:x}");
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
        let cache_key = format!("get_account_{block:x}_{address:x}");

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
    /// limiting, and transport for both single requests and batches.
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
        if let Some(ref limiter) = self.inner.limiter {
            let count = request_payload.as_array().map(|a| a.len()).unwrap_or(1);
            trace!(%cache_key, count, "rate limit acquire");
            for _ in 0..count {
                limiter.acquire();
            }
        }

        // 4. Live network fetch
        let transport_result = if let Some(items) = request_payload.as_array() {
            self.inner
                .transport
                .send_batch(items.to_vec())
                .map(serde_json::Value::Array)
                .with_context(|| format!("RPC batch {cache_key} failed"))
        } else {
            self.inner
                .transport
                .send(request_payload)
                .with_context(|| format!("RPC {cache_key} failed"))
        };

        let result = match transport_result {
            Ok(r) => r,
            Err(e) => {
                self.inner
                    .dedup
                    .complete(cache_key, Err(anyhow::anyhow!("{e}")));
                guard.deactivate();
                return Err(e);
            }
        };

        // 4b. Reject batch responses that contain any RPC error.
        // We must not cache an errored batch because a retry might succeed.
        if let Some(arr) = result.as_array() {
            for item in arr {
                if let Some(error) = item.get("error") {
                    let err = anyhow::anyhow!("RPC error in batch response: {error}");
                    self.inner
                        .dedup
                        .complete(cache_key, Err(anyhow::anyhow!("{err}")));
                    guard.deactivate();
                    return Err(err);
                }
            }
        }

        // 5. Update cache and complete dedup
        if !skip_cache && let Some(ref cache) = self.inner.cache {
            cache.insert(cache_key, result.clone());
        }
        self.inner.dedup.complete(cache_key, Ok(result.clone()));
        guard.deactivate();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    #[test]
    fn agent_pool_round_robin() {
        let a1 = ureq::Agent::new_with_defaults();
        let a2 = ureq::Agent::new_with_defaults();
        let pool = AgentPool::new(vec![a1, a2]);

        let first = pool.next() as *const Agent;
        let second = pool.next() as *const Agent;
        let third = pool.next() as *const Agent;

        assert_ne!(first, second);
        assert_eq!(first, third);
    }

    #[test]
    fn url_pool_round_robin() {
        let pool = UrlPool::new(vec![
            "https://a.example.com".into(),
            "https://b.example.com".into(),
        ]);

        assert_eq!(pool.next(), "https://a.example.com");
        assert_eq!(pool.next(), "https://b.example.com");
        assert_eq!(pool.next(), "https://a.example.com");
    }

    #[test]
    fn rpc_cache_key_returns_chain_id() {
        let config = Config::new()
            .urls(vec!["http://localhost:1".into()])
            .pool_size(1)
            .chain_id(8453);
        let rpc = Client::new(config);
        assert_eq!(rpc.cache_key(), "8453");
    }

    #[test]
    fn mock_transport_roundtrip() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.insert(
            "eth_blockNumber",
            &[],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );

        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
        let rpc = Client::new_with_transport(config, transport);

        let result = rpc.latest_block_number().unwrap();
        assert_eq!(result, 0x1a2b);
    }

    #[test]
    fn dedup_coalesces_parallel_requests() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.set_delay(Duration::from_millis(100));
        transport.insert(
            "eth_blockNumber",
            &[],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );

        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
        let rpc = Client::new_with_transport(config, transport.clone());
        let rpc2 = rpc.clone();

        let t1 = std::thread::spawn(move || rpc.latest_block_number());
        let t2 = std::thread::spawn(move || rpc2.latest_block_number());

        let r1 = t1.join().unwrap().unwrap();
        let r2 = t2.join().unwrap().unwrap();
        assert_eq!(r1, r2);
        assert_eq!(r1, 0x1a2b);
        assert_eq!(transport.call_count("eth_blockNumber", &[]), 1);
    }

    #[test]
    fn rate_limit_throttles_without_network() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.insert(
            "eth_blockNumber",
            &[],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1"}),
        );

        let config = Config::new()
            .urls(vec!["mock://test".into()])
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

    /// Stress-test deduplication with many threads hitting the same request.
    /// We assert deduplication via [`MockTransport::call_count`]:
    /// if N parallel threads coalesce, the transport sees exactly 1 dispatch.
    #[test]
    fn dedup_coalesces_many_parallel_requests() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.set_delay(Duration::from_millis(100));
        transport.insert(
            "eth_blockNumber",
            &[],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );

        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
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
        assert_eq!(transport.call_count("eth_blockNumber", &[]), 1);
    }

    /// Use a barrier to release all threads at the exact same instant,
    /// maximizing the race window and proving the dedup table is sound.
    #[test]
    fn dedup_with_barrier_maximizes_contention() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.set_delay(Duration::from_millis(200));
        transport.insert(
            "eth_blockNumber",
            &[],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0xdeadbeef"}),
        );

        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
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
        assert_eq!(transport.call_count("eth_blockNumber", &[]), 1);
    }

    /// Verify that deduplication is keyed by (method, params).
    /// Two distinct requests issued from parallel threads must each
    /// be dispatched exactly once.
    #[test]
    fn dedup_only_coalesces_identical_requests() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.set_delay(Duration::from_millis(50));
        transport.insert(
            "eth_getBalance",
            &[
                json!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                json!("0x112233"),
            ],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1"}),
        );
        transport.insert(
            "eth_getBalance",
            &[
                json!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                json!("0x112233"),
            ],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x2"}),
        );

        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
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

        assert_eq!(
            transport.call_count(
                "eth_getBalance",
                &[
                    json!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                    json!("0x112233")
                ]
            ),
            1
        );
        assert_eq!(
            transport.call_count(
                "eth_getBalance",
                &[
                    json!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                    json!("0x112233")
                ]
            ),
            1
        );
    }

    /// Rate limit + dedup interaction: the first batch consumes the initial
    /// token bucket. A second parallel batch for the *same* request must still
    /// wait for the rate-limit refill before the leader can dispatch.
    ///
    /// Asserted via [`MockTransport`]:
    /// - `call_count == 2` (one dispatch per batch, dedup coalesces each batch).
    /// - second-batch elapsed >= 800 ms (proves the leader was throttled).
    #[test]
    fn rate_limit_throttles_parallel_deduped_requests() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.set_delay(Duration::from_millis(50));
        transport.insert(
            "eth_blockNumber",
            &[],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1"}),
        );

        let config = Config::new()
            .urls(vec!["mock://test".into()])
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
        assert_eq!(transport.call_count("eth_blockNumber", &[]), 2);
    }

    /// Maximal contention: a barrier releases many threads at the exact same
    /// instant *after* the token bucket is empty. Only one dispatch happens,
    /// but that dispatch is delayed by the rate limiter.
    #[test]
    fn rate_limit_with_barrier_and_dedup() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.set_delay(Duration::from_millis(50));
        transport.insert(
            "eth_blockNumber",
            &[],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0xcafe"}),
        );

        let config = Config::new()
            .urls(vec!["mock://test".into()])
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
        assert_eq!(transport.call_count("eth_blockNumber", &[]), 2);
    }

    /// Verify that `eth_blockNumber` bypasses the cache while other
    /// methods are still cached. Sequential calls to `latest_block_number`
    /// must hit the transport every time, whereas `get_balance` should be
    /// served from cache on the second call.
    #[test]
    fn eth_block_number_skips_cache_but_other_methods_are_cached() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.insert(
            "eth_blockNumber",
            &[],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x1a2b"}),
        );
        transport.insert(
            "eth_getBalance",
            &[
                json!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                json!("0x1"),
            ],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0xc0ffee"}),
        );

        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new()
            .urls(vec!["mock://test".into()])
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

        assert_eq!(transport.call_count("eth_blockNumber", &[]), 2);
        assert_eq!(
            transport.call_count(
                "eth_getBalance",
                &[
                    json!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                    json!("0x1")
                ]
            ),
            1
        );
    }

    #[test]
    fn mock_get_account_roundtrip() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.insert(
            "eth_getBalance",
            &[
                json!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"),
                json!("0x17fa30b"),
            ],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x4ec7cefe1a0664fd"}),
        );
        transport.insert(
            "eth_getTransactionCount",
            &[
                json!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"),
                json!("0x17fa30b"),
            ],
            json!({"jsonrpc": "2.0", "id": 2, "result": "0x1707"}),
        );
        transport.insert(
            "eth_getCode",
            &[
                json!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"),
                json!("0x17fa30b"),
            ],
            json!({"jsonrpc": "2.0", "id": 3, "result": "0x6060604052"}),
        );

        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
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
        assert_eq!(
            transport.call_count(
                "eth_getBalance",
                &[
                    json!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"),
                    json!("0x17fa30b"),
                ]
            ),
            1
        );
        assert_eq!(
            transport.call_count(
                "eth_getTransactionCount",
                &[
                    json!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"),
                    json!("0x17fa30b"),
                ]
            ),
            1
        );
        assert_eq!(
            transport.call_count(
                "eth_getCode",
                &[
                    json!("0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2"),
                    json!("0x17fa30b"),
                ]
            ),
            1
        );
    }

    // -----------------------------------------------------------------
    // Issue #1 reproduction: batch response ordering & error caching
    // -----------------------------------------------------------------

    /// Transport that returns batch responses in *reverse* order,
    /// echoing back the request `id` like a real JSON-RPC node.
    #[derive(Debug)]
    struct ReversedBatchTransport;

    impl crate::rpc_v2::transport::Transport for ReversedBatchTransport {
        fn send(&self, _payload: serde_json::Value) -> Result<serde_json::Value> {
            unimplemented!("use send_batch")
        }

        fn send_batch(&self, payloads: Vec<serde_json::Value>) -> Result<Vec<serde_json::Value>> {
            let mut responses = Vec::with_capacity(payloads.len());
            for payload in &payloads {
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
            Ok(responses)
        }
    }

    // Regression test: batch responses may arrive out-of-order.
    // `get_account` must match by `id`, not by array position.
    #[test]
    fn get_account_matches_responses_by_id() {
        let transport = Arc::new(ReversedBatchTransport);
        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
        let rpc = Client::new_with_transport(config, transport);

        let addr: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();
        let (balance, nonce, code) = rpc.get_account(addr, 1).unwrap();

        // Even though the transport reversed the order, the correct values
        // must be extracted by matching the `id` field in each response.
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
    #[derive(Debug)]
    struct ErrorThenSuccessTransport(std::sync::atomic::AtomicUsize);

    impl crate::rpc_v2::transport::Transport for ErrorThenSuccessTransport {
        fn send(&self, _payload: serde_json::Value) -> Result<serde_json::Value> {
            unimplemented!("use send_batch")
        }

        fn send_batch(&self, payloads: Vec<serde_json::Value>) -> Result<Vec<serde_json::Value>> {
            let count = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let ids: Vec<serde_json::Value> = payloads
                .iter()
                .map(|p| p.get("id").cloned().unwrap_or(json!(0)))
                .collect();
            if count == 0 {
                // First call: balance item is an RPC error.
                Ok(vec![
                    json!({"jsonrpc":"2.0","id":ids[0],"error":{"code":-32000,"message":"rate limited"}}),
                    json!({"jsonrpc":"2.0","id":ids[1],"result":"0x1"}),
                    json!({"jsonrpc":"2.0","id":ids[2],"result":"0x6000"}),
                ])
            } else {
                // Second call: clean data.
                Ok(vec![
                    json!({"jsonrpc":"2.0","id":ids[0],"result":"0xDEADBEEF"}),
                    json!({"jsonrpc":"2.0","id":ids[1],"result":"0x1"}),
                    json!({"jsonrpc":"2.0","id":ids[2],"result":"0x6000"}),
                ])
            }
        }
    }

    // Regression test: a batch containing an RPC error must NOT be cached.
    // The second call should trigger a fresh request and succeed.
    #[test]
    fn get_account_batch_error_is_not_cached() {
        let transport = Arc::new(ErrorThenSuccessTransport(
            std::sync::atomic::AtomicUsize::new(0),
        ));
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new()
            .urls(vec!["mock://test".into()])
            .chain_id(1)
            .cache_dir(tmp.path());
        let rpc = Client::new_with_transport(config, transport.clone());

        let addr: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse()
            .unwrap();

        // First call fails because the balance item carries an RPC error.
        let err = rpc.get_account(addr, 1).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("rate limited"),
            "first call should fail with the RPC error message: {msg}"
        );

        // Second call must NOT hit the cache. It should issue a fresh request
        // and receive the clean data from the transport.
        let (balance, nonce, code) = rpc.get_account(addr, 1).unwrap();
        assert_eq!(balance, U256::from(0xDEADBEEFu64));
        assert_eq!(nonce, 1);
        assert_eq!(code, "0x6000".parse::<Bytes>().unwrap());

        // The transport was called exactly twice.
        assert_eq!(transport.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    fn live_client() -> &'static Client {
        static LIVE_CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();
        LIVE_CLIENT.get_or_init(|| {
            let url = std::env::var("RAPTOR_RPC_URL")
                .expect("RAPTOR_RPC_URL must be set to run live tests");
            let config = Config::new().urls(vec![url]).chain_id(1).pool_size(1);
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
