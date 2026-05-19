//! Dedicated RPC module: connection pooling, request deduplication, retries,
//! backoff, and optional rate limiting for JSON-RPC endpoints.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use tracing::{instrument, trace};

/// Trait for anything that can execute a JSON-RPC method call.
pub trait RpcClient: Send + Sync + std::fmt::Debug {
    fn call(&self, method: &str, params: &[serde_json::Value]) -> Result<serde_json::Value>;
    fn latest_block_number(&self) -> Result<u64>;
    fn cache_key(&self) -> String;
}

impl RpcClient for Rpc {
    fn call(&self, method: &str, params: &[serde_json::Value]) -> Result<serde_json::Value> {
        self.call(method, params)
    }
    fn latest_block_number(&self) -> Result<u64> {
        self.latest_block_number()
    }
    fn cache_key(&self) -> String {
        self.config().urls.first().cloned().unwrap_or_default()
    }
}

use client::{AgentPool, UrlPool};
use dedup::{DedupTable, RequestKey};
use limiter::RateLimiter;
pub use mock::FakeRpc;

mod client;
mod dedup;
mod limiter;
mod mock;
mod request;

/// Serializable configuration for RPC transport.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RpcConfig {
    /// JSON-RPC endpoints for the same chain. The first URL is the default
    /// primary; all entries are treated equally for load balancing and failover.
    pub urls: Vec<String>,
    /// Number of `ureq::Agent` instances in the pool.
    pub pool_size: u32,
    /// Total timeout for a single RPC call (milliseconds).
    pub timeout_ms: u64,
    /// Maximum retry attempts per URL after transient failure.
    pub retries: u32,
    /// Initial retry backoff in milliseconds (doubles each attempt).
    pub retry_backoff_ms: u64,
    /// Optional rate limit: maximum requests per second across all URLs.
    pub requests_per_second: Option<u64>,
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
        }
    }
}

/// Synchronous JSON-RPC client with pooling, deduplication, retries, and
/// optional rate limiting.
#[derive(Debug)]
pub struct Rpc {
    config: RpcConfig,
    urls: UrlPool,
    pool: AgentPool,
    dedup: DedupTable,
    limiter: Option<RateLimiter>,
}

impl Rpc {
    /// Start a builder from one or more URLs.
    pub fn with_urls(urls: &[String]) -> RpcBuilder {
        RpcBuilder {
            config: RpcConfig::with_urls(urls.to_vec()),
        }
    }

    /// Create a no-op RPC instance for backends that never touch the network.
    ///
    /// This is used by `ForkDB::empty()`.  The returned `Rpc` has a
    /// dummy URL and is never actually invoked because `is_empty == true`
    /// short-circuits every `DatabaseRef` method.
    pub fn noop() -> Self {
        Self {
            config: RpcConfig::with_urls(vec!["http://localhost:1".to_string()]),
            urls: UrlPool::new(vec!["http://localhost:1".to_string()]),
            pool: AgentPool::new(vec![ureq::Agent::new_with_defaults()]),
            dedup: DedupTable::new(),
            limiter: None,
        }
    }

    /// Access the underlying configuration.
    pub fn config(&self) -> &RpcConfig {
        &self.config
    }

    /// Execute a single JSON-RPC method with deduplication, rate limiting,
    /// retries, round-robin URL selection, and round-robin agent selection.
    #[instrument(skip(self, params), fields(method))]
    pub fn call(&self, method: &str, params: &[serde_json::Value]) -> Result<serde_json::Value> {
        let key = RequestKey::new(method, params);

        // 1. Deduplication
        if let Some(result) = self.dedup.register(&key) {
            trace!(%key, "dedup hit — waiting on in-flight request");
            return result;
        }

        let _guard = self.dedup.guard(&key);

        // 2. Rate limit gate
        if let Some(ref limiter) = self.limiter {
            trace!(%key, "rate limit acquire");
            limiter.acquire();
        }

        // 3. Serialize payload
        let payload = request::payload(method, params);
        let body = serde_json::to_vec(&payload).context("serializing RPC payload")?;

        // 4. Outer loop: try each URL (failover). Inner loop: retry same URL.
        let url_count = self.urls.urls().len();
        let mut last_err: Option<anyhow::Error> = None;

        for url_idx in 0..url_count {
            let url = self.urls.next();
            trace!(method, %url, url_idx, "selected RPC URL");

            for attempt in 0..self.config.retries {
                let agent = self.pool.next();
                trace!(method, %url, attempt, "sending RPC request");

                match agent
                    .post(url)
                    .header("Content-Type", "application/json")
                    .send(&body)
                {
                    Ok(mut response) => {
                        let text = response
                            .body_mut()
                            .read_to_string()
                            .context("reading RPC response body")?;
                        let value: serde_json::Value =
                            serde_json::from_str(&text).context("json decode")?;

                        match value {
                            serde_json::Value::Object(ref map) if map.get("error").is_some() => {
                                let err = &map["error"];
                                trace!(method, %url, attempt, %err, "RPC error response");
                                last_err = Some(anyhow::anyhow!("RPC error: {err}"));
                            }
                            serde_json::Value::Object(mut map) => {
                                if let Some(result) = map.remove("result") {
                                    return self
                                        .complete_success(&key, _guard, result, method, url);
                                }
                                last_err = Some(anyhow::anyhow!("missing result field"));
                            }
                            _ => {
                                last_err = Some(anyhow::anyhow!("missing result field"));
                            }
                        }
                    }
                    Err(e) => {
                        trace!(method, %url, attempt, %e, "RPC request failed (transient)");
                        last_err = Some(anyhow::Error::new(e));
                    }
                }

                let backoff =
                    Duration::from_millis(self.config.retry_backoff_ms * (attempt + 1) as u64);
                trace!(method, %url, attempt, ?backoff, "backing off before retry");
                std::thread::sleep(backoff);
            }

            trace!(method, %url, "exhausted retries on this URL, trying next");
        }

        let err = last_err.unwrap_or_else(|| anyhow::anyhow!("RPC request failed on all URLs"));
        trace!(method, "RPC request exhausted all URLs");
        self.dedup.complete(&key, Err(anyhow::anyhow!("{err}")));
        _guard.deactivate();
        Err(err)
    }

    fn complete_success(
        &self,
        key: &RequestKey,
        guard: dedup::DedupGuard<'_>,
        result: serde_json::Value,
        method: &str,
        url: &str,
    ) -> Result<serde_json::Value> {
        trace!(method, %url, "RPC request succeeded");
        let r = result.clone();
        self.dedup.complete(key, Ok(result));
        guard.deactivate();
        Ok(r)
    }

    /// Query the remote node for its latest block number.
    pub fn latest_block_number(&self) -> Result<u64> {
        let result = self.call("eth_blockNumber", &[])?;
        let s = result.as_str().context("missing result in RPC response")?;
        let s = s.strip_prefix("0x").unwrap_or(s);
        u64::from_str_radix(s, 16).context("invalid block number")
    }
}

/// Builder for [`Rpc`].
#[derive(Debug, Clone)]
pub struct RpcBuilder {
    config: RpcConfig,
}

impl RpcBuilder {
    /// Set the agent pool size.
    pub fn with_pool_size(mut self, size: u32) -> Self {
        self.config.pool_size = size.max(1);
        self
    }

    /// Set the total timeout per request.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout_ms = timeout.as_millis() as u64;
        self
    }

    /// Set the maximum retry attempts per URL.
    pub fn with_retries(mut self, retries: u32) -> Self {
        self.config.retries = retries;
        self
    }

    /// Set the initial retry backoff.
    pub fn with_retry_backoff(mut self, backoff: Duration) -> Self {
        self.config.retry_backoff_ms = backoff.as_millis() as u64;
        self
    }

    /// Set the optional rate limit in requests per second.
    pub fn with_requests_per_second(mut self, rate: Option<u64>) -> Self {
        self.config.requests_per_second = rate;
        self
    }

    /// Build the [`Rpc`] instance.
    pub fn build(self) -> Result<Rpc> {
        ensure!(
            !self.config.urls.is_empty(),
            "at least one RPC URL is required"
        );
        ensure!(
            self.config.requests_per_second != Some(0),
            "rate limit must be > 0"
        );
        ensure!(
            self.config.retry_backoff_ms > 0,
            "retry backoff must be > 0"
        );

        let pool_size = self.config.pool_size.max(1) as usize;
        let timeout = Duration::from_millis(self.config.timeout_ms);

        let agents: Vec<ureq::Agent> = (0..pool_size)
            .map(|_| {
                let cfg = ureq::Agent::config_builder()
                    .timeout_global(Some(timeout))
                    .build();
                ureq::Agent::new_with_config(cfg)
            })
            .collect();

        let limiter = self.config.requests_per_second.map(RateLimiter::new);

        Ok(Rpc {
            config: self.config.clone(),
            urls: UrlPool::new(self.config.urls),
            pool: AgentPool::new(agents),
            dedup: DedupTable::new(),
            limiter,
        })
    }
}
