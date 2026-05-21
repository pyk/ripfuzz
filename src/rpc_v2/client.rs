//! HTTP agent pool, URL pool, and JSON-RPC client with caching, deduplication,
//! rate limiting, retries, and failover.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use alloy_primitives::{Address, B256, U256};
use anyhow::{Context, Result, bail};
use serde_json::json;
use tracing::{instrument, trace};
use ureq::Agent;

use crate::rpc_v2::config::Config;
use crate::rpc_v2::request;
use crate::rpc_v2::transport::{HttpTransport, Transport};
use crate::rpc_v2::{Cache, DedupTable, RateLimiter, RequestKey};

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
#[derive(Debug, Clone)]
pub struct Block {
    pub number: u64,
    pub timestamp: u64,
    pub coinbase: Address,
    pub gas_limit: u64,
    pub basefee: u64,
    pub prevrandao: Option<B256>,
    pub difficulty: U256,
    pub hash: Option<B256>,
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
        let cache = config.cache_dir.map(|dir| {
            let path = dir
                .join(format!("{}", config.chain_id))
                .join("rpc_cache.json");
            Cache::new(path)
        });
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
        let result = self.call("eth_blockNumber", &[])?;
        let s = result.as_str().context("missing result in RPC response")?;
        let s = s.strip_prefix("0x").unwrap_or(s);
        u64::from_str_radix(s, 16).context("invalid block number")
    }

    /// Fetch a block header by number.
    pub fn get_block_by_number(&self, block: u64) -> Result<Block> {
        let block_hex = format!("0x{:x}", block);
        let result = self.call("eth_getBlockByNumber", &[json!(block_hex), json!(false)])?;
        if result.is_null() {
            bail!("block {} not found", block);
        }
        parse_block(&result)
    }

    /// Fetch an account balance at a specific block.
    pub fn get_balance(&self, address: Address, block: u64) -> Result<U256> {
        let result = self.call(
            "eth_getBalance",
            &[
                json!(format!("0x{address:x}")),
                json!(format!("0x{:x}", block)),
            ],
        )?;
        parse_u256(&result)
    }

    /// Fetch an account nonce at a specific block.
    pub fn get_transaction_count(&self, address: Address, block: u64) -> Result<u64> {
        let result = self.call(
            "eth_getTransactionCount",
            &[
                json!(format!("0x{address:x}")),
                json!(format!("0x{:x}", block)),
            ],
        )?;
        parse_u64(&result)
    }

    /// Fetch contract bytecode at a specific block.
    pub fn get_code(&self, address: Address, block: u64) -> Result<Vec<u8>> {
        let result = self.call(
            "eth_getCode",
            &[
                json!(format!("0x{address:x}")),
                json!(format!("0x{:x}", block)),
            ],
        )?;
        parse_hex_bytes(&result)
    }

    /// Fetch a storage slot at a specific block.
    pub fn get_storage_at(&self, address: Address, slot: U256, block: u64) -> Result<U256> {
        let result = self.call(
            "eth_getStorageAt",
            &[
                json!(format!("0x{address:x}")),
                json!(format!("0x{slot:x}")),
                json!(format!("0x{:x}", block)),
            ],
        )?;
        parse_u256(&result)
    }

    /// Explicitly flush the in-memory cache to disk.
    pub fn flush_cache(&self) -> Result<()> {
        if let Some(ref cache) = self.inner.cache {
            cache.flush()?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    // Internal call pipeline
    // -----------------------------------------------------------------

    #[instrument(skip(self, params), fields(method))]
    fn call(&self, method: &str, params: &[serde_json::Value]) -> Result<serde_json::Value> {
        let key = RequestKey::new(method, params);

        // 1. Deduplication
        if let Some(result) = self.inner.dedup.register(&key) {
            trace!(%key, "dedup hit");
            return result;
        }
        let guard = self.inner.dedup.guard(&key);

        // 2. Memory cache (seeded from disk at construction)
        if let Some(ref cache) = self.inner.cache
            && let Some(value) = cache.get(&key)
        {
            trace!(%key, "cache hit");
            self.inner.dedup.complete(&key, Ok(value.clone()));
            guard.deactivate();
            return Ok(value);
        }

        // 3. Rate limit gate
        if let Some(ref limiter) = self.inner.limiter {
            trace!(%key, "rate limit acquire");
            limiter.acquire();
        }

        // 4. Live network fetch
        let payload = request::payload(method, params);
        let response = self
            .inner
            .transport
            .send(payload)
            .with_context(|| format!("RPC {method} failed"))?;

        // 5. Update cache layers
        if let Some(ref cache) = self.inner.cache {
            cache.insert(key.clone(), response.clone());
        }

        // 6. Complete dedup and return
        self.inner.dedup.complete(&key, Ok(response.clone()));
        guard.deactivate();
        Ok(response)
    }
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        if let Some(ref cache) = self.cache {
            let _ = cache.flush();
        }
    }
}

// -----------------------------------------------------------------
// Parsers
// -----------------------------------------------------------------

fn parse_u256(value: &serde_json::Value) -> Result<U256> {
    let s = value.as_str().context("expected hex string")?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    U256::from_str_radix(s, 16).context("invalid u256 hex")
}

fn parse_u64(value: &serde_json::Value) -> Result<u64> {
    let s = value.as_str().context("expected hex string")?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(s, 16).context("invalid u64 hex")
}

fn parse_hex_bytes(value: &serde_json::Value) -> Result<Vec<u8>> {
    let s = value.as_str().context("expected hex string")?;
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).context("invalid hex bytes")
}

fn parse_block(value: &serde_json::Value) -> Result<Block> {
    let number = parse_u64_field(value, "number")?.unwrap_or(0);
    let timestamp = parse_u64_field(value, "timestamp")?.unwrap_or(0);
    let coinbase = parse_address_field(value, "miner")?.unwrap_or(Address::ZERO);
    let gas_limit = parse_u64_field(value, "gasLimit")?.unwrap_or(30_000_000);
    let basefee = parse_u64_field(value, "baseFeePerGas")?.unwrap_or(0);
    let difficulty = parse_u256_field(value, "difficulty")?.unwrap_or(U256::ZERO);
    let prevrandao = parse_b256_field(value, "mixHash").ok();
    let hash = parse_b256_field(value, "hash").ok();

    Ok(Block {
        number,
        timestamp,
        coinbase,
        gas_limit,
        basefee,
        prevrandao,
        difficulty,
        hash,
    })
}

fn parse_u64_field(value: &serde_json::Value, key: &str) -> Result<Option<u64>> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| {
            let s = s.strip_prefix("0x").unwrap_or(s);
            u64::from_str_radix(s, 16).with_context(|| format!("invalid {key}"))
        })
        .transpose()
}

fn parse_u256_field(value: &serde_json::Value, key: &str) -> Result<Option<U256>> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| {
            let s = s.strip_prefix("0x").unwrap_or(s);
            U256::from_str_radix(s, 16).with_context(|| format!("invalid {key}"))
        })
        .transpose()
}

fn parse_address_field(value: &serde_json::Value, key: &str) -> Result<Option<Address>> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.parse().with_context(|| format!("invalid {key}")))
        .transpose()
}

fn parse_b256_field(value: &serde_json::Value, key: &str) -> Result<B256> {
    let s = value
        .get(key)
        .and_then(|v| v.as_str())
        .with_context(|| format!("missing {key}"))?;
    s.parse().with_context(|| format!("invalid {key}"))
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
        transport.insert("eth_blockNumber", &[], "0x1a2b".into());

        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
        let rpc = Client::new_with_transport(config, transport);

        let result = rpc.latest_block_number().unwrap();
        assert_eq!(result, 0x1a2b);
    }

    #[test]
    fn dedup_coalesces_parallel_requests() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.set_delay(Duration::from_millis(100));
        transport.insert("eth_blockNumber", &[], "0x1a2b".into());

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
        transport.insert("eth_blockNumber", &[], "0x1".into());

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

    #[test]
    fn parse_u256_valid() {
        let v = serde_json::Value::String("0x1a2b".into());
        assert_eq!(parse_u256(&v).unwrap(), U256::from(0x1a2bu64));
    }

    #[test]
    fn parse_u64_valid() {
        let v = serde_json::Value::String("0x10".into());
        assert_eq!(parse_u64(&v).unwrap(), 16u64);
    }

    #[test]
    fn parse_hex_bytes_valid() {
        let v = serde_json::Value::String("0xabcd".into());
        assert_eq!(parse_hex_bytes(&v).unwrap(), vec![0xab, 0xcd]);
    }

    /// Stress-test deduplication with many threads hitting the same request.
    /// We assert deduplication via [`MockTransport::call_count`]:
    /// if N parallel threads coalesce, the transport sees exactly 1 dispatch.
    #[test]
    fn dedup_coalesces_many_parallel_requests() {
        let transport = Arc::new(crate::rpc_v2::transport::MockTransport::default());
        transport.set_delay(Duration::from_millis(100));
        transport.insert("eth_blockNumber", &[], "0x1a2b".into());

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
        transport.insert("eth_blockNumber", &[], "0xdeadbeef".into());

        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
        let rpc = Client::new_with_transport(config, transport.clone());

        let thread_count = 20;
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
            "0x1".into(),
        );
        transport.insert(
            "eth_getBalance",
            &[
                json!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
                json!("0x112233"),
            ],
            "0x2".into(),
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
}
