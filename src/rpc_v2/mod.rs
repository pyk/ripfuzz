//! RPC v2: self-contained JSON-RPC client with 2-layer caching, deduplication,
//! rate limiting, retries, and typed EVM method wrappers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Address, B256, U256};
use anyhow::{Context, Result, bail, ensure};
use serde_json::json;
use tracing::{instrument, trace};

pub use cache::Cache;
pub use client::{AgentPool, UrlPool};
pub use dedup::{DedupTable, RequestKey};
pub use limiter::RateLimiter;
pub use transport::{HttpTransport, MockTransport, Transport};

mod cache;
mod client;
mod dedup;
mod limiter;
mod request;
pub mod transport;

/// Serializable configuration for RPC transport.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RpcConfig {
    /// JSON-RPC endpoints for the same chain.
    pub urls: Vec<String>,
    /// Number of concurrent HTTP agents in the pool.
    pub pool_size: u32,
    /// Total timeout for a single RPC call (milliseconds).
    pub timeout_ms: u64,
    /// Maximum retry attempts per URL after transient failure.
    pub retries: u32,
    /// Initial retry backoff in milliseconds (doubles each attempt).
    pub retry_backoff_ms: u64,
    /// Optional rate limit: maximum requests per second across all URLs.
    pub requests_per_second: Option<u64>,
    /// Chain ID used for cache key derivation.
    pub chain_id: Option<u64>,
    /// Directory for the disk cache layer.
    pub cache_dir: Option<PathBuf>,
}

impl RpcConfig {
    /// Return defaults with the given URLs.
    pub fn with_urls(urls: Vec<String>) -> Self {
        Self {
            urls,
            pool_size: 4,
            timeout_ms: 30_000,
            retries: 3,
            retry_backoff_ms: 100,
            requests_per_second: None,
            chain_id: None,
            cache_dir: None,
        }
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
pub struct Rpc {
    inner: Arc<RpcInner>,
}

#[derive(Debug)]
struct RpcInner {
    transport: Arc<dyn Transport>,
    cache: Option<Cache>,
    dedup: DedupTable,
    limiter: Option<RateLimiter>,
    chain_id: u64,
}

impl Rpc {
    /// Start a builder from one or more URLs.
    pub fn with_urls(urls: &[String]) -> RpcBuilder {
        RpcBuilder::with_urls(urls)
    }

    /// Cache key derived from the configured chain ID.
    pub fn cache_key(&self) -> String {
        format!("{}", self.inner.chain_id)
    }

    /// Configured chain ID.
    pub fn chain_id(&self) -> u64 {
        self.inner.chain_id
    }

    // -----------------------------------------------------------------
    // Validation
    // -----------------------------------------------------------------

    /// Validate that all configured URLs are reachable and report the same
    /// chain ID as the one configured at build time.
    pub fn validate_chain_id(&self) -> Result<u64> {
        let validated = self.inner.transport.validate_chain_id()?;
        ensure!(
            validated == self.inner.chain_id,
            "configured chain_id {} does not match validated chain_id {}",
            self.inner.chain_id,
            validated
        );
        Ok(validated)
    }

    /// Query the remote node for its latest block number.
    pub fn latest_block_number(&self) -> Result<u64> {
        let result = self.call("eth_blockNumber", &[])?;
        let s = result.as_str().context("missing result in RPC response")?;
        let s = s.strip_prefix("0x").unwrap_or(s);
        u64::from_str_radix(s, 16).context("invalid block number")
    }

    // -----------------------------------------------------------------
    // Typed EVM methods
    // -----------------------------------------------------------------

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

    /// Explicitly flush the in-memory cache to disk.
    pub fn flush_cache(&self) -> Result<()> {
        if let Some(ref cache) = self.inner.cache {
            cache.flush()?;
        }
        Ok(())
    }
}

impl Drop for RpcInner {
    fn drop(&mut self) {
        if let Some(ref cache) = self.cache {
            let _ = cache.flush();
        }
    }
}

// -----------------------------------------------------------------
// Builder
// -----------------------------------------------------------------

/// Builder for [`Rpc`].
#[derive(Debug, Clone)]
pub struct RpcBuilder {
    config: RpcConfig,
    transport: Option<Arc<dyn Transport>>,
}

impl RpcBuilder {
    pub fn with_urls(urls: &[String]) -> Self {
        Self {
            config: RpcConfig::with_urls(urls.to_vec()),
            transport: None,
        }
    }

    /// Provide a custom transport (e.g. [`MockTransport`] for testing).
    pub fn with_transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = Some(transport);
        self
    }

    pub fn with_pool_size(mut self, size: u32) -> Self {
        self.config.pool_size = size.max(1);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout_ms = timeout.as_millis() as u64;
        self
    }

    pub fn with_retries(mut self, retries: u32) -> Self {
        self.config.retries = retries;
        self
    }

    pub fn with_retry_backoff(mut self, backoff: Duration) -> Self {
        self.config.retry_backoff_ms = backoff.as_millis() as u64;
        self
    }

    pub fn with_requests_per_second(mut self, rate: Option<u64>) -> Self {
        self.config.requests_per_second = rate;
        self
    }

    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.config.chain_id = Some(chain_id);
        self
    }

    pub fn with_cache_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.config.cache_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    pub fn build(self) -> Result<Rpc> {
        if self.transport.is_none() {
            ensure!(
                !self.config.urls.is_empty(),
                "at least one RPC URL is required"
            );
        }
        ensure!(
            self.config.retry_backoff_ms > 0,
            "retry backoff must be > 0"
        );
        if let Some(rate) = self.config.requests_per_second {
            ensure!(rate > 0, "rate limit must be > 0");
        }

        let chain_id = self.config.chain_id.unwrap_or(0);

        let transport = self.transport.unwrap_or_else(|| {
            let timeout = Duration::from_millis(self.config.timeout_ms);
            let pool_size = self.config.pool_size.max(1) as usize;
            let agents: Vec<ureq::Agent> = (0..pool_size)
                .map(|_| {
                    let cfg = ureq::Agent::config_builder()
                        .timeout_global(Some(timeout))
                        .build();
                    ureq::Agent::new_with_config(cfg)
                })
                .collect();
            Arc::new(HttpTransport::new(
                self.config.urls.clone(),
                agents,
                self.config.retries,
                Duration::from_millis(self.config.retry_backoff_ms),
            ))
        });

        let limiter = self.config.requests_per_second.map(RateLimiter::new);

        let cache = self.config.cache_dir.map(|dir| {
            let path = dir.join(format!("{}", chain_id)).join("rpc_cache.json");
            Cache::new(path)
        });

        Ok(Rpc {
            inner: Arc::new(RpcInner {
                transport,
                cache,
                dedup: DedupTable::new(),
                limiter,
                chain_id,
            }),
        })
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

    use super::*;

    #[test]
    fn rpc_cache_key_returns_chain_id() {
        let rpc = Rpc::with_urls(&["http://localhost:1".into()])
            .with_pool_size(1)
            .with_chain_id(8453)
            .build()
            .unwrap();
        assert_eq!(rpc.cache_key(), "8453");
    }

    #[test]
    fn mock_transport_roundtrip() {
        let transport = Arc::new(MockTransport::default());
        transport.set_chain_id(1);
        transport.insert("eth_blockNumber", &[], "0x1a2b".into());

        let rpc = Rpc::with_urls(&["mock://test".into()])
            .with_transport(transport)
            .with_chain_id(1)
            .build()
            .unwrap();

        let result = rpc.latest_block_number().unwrap();
        assert_eq!(result, 0x1a2b);
    }

    #[test]
    fn dedup_coalesces_parallel_requests() {
        let transport = Arc::new(MockTransport::default());
        transport.set_chain_id(1);
        transport.set_delay(Duration::from_millis(100));
        transport.insert("eth_blockNumber", &[], "0x1a2b".into());

        let rpc = Rpc::with_urls(&["mock://test".into()])
            .with_transport(transport.clone())
            .with_chain_id(1)
            .build()
            .unwrap();

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
        let transport = Arc::new(MockTransport::default());
        transport.set_chain_id(1);
        transport.insert("eth_blockNumber", &[], "0x1".into());

        let rpc = Rpc::with_urls(&["mock://test".into()])
            .with_transport(transport)
            .with_chain_id(1)
            .with_requests_per_second(Some(2))
            .build()
            .unwrap();

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
}
