//! RPC configuration for fork mode.
//!
//! This module provides [`Config`], a builder-style struct that holds all
//! parameters needed to connect to a remote JSON-RPC node for state forking.
//!
//! [`Config`] can validate its own consistency via [`Config::validate`],
//! which checks that a URL and a fork block are configured, and that the
//! URL reports the same `chain_id` as the one configured (defaulting to `1`).
//!
//! # Example
//!
//! ```rust,no_run
//! # fn main() -> anyhow::Result<()> {
//! use raptor::rpc_v2::Config;
//!
//! let config = Config::new()
//!     .url("https://mainnet.example.com")
//!     .block(18_000_000)
//!     .chain_id(1);
//!
//! config.validate()?;
//! # Ok(())
//! # }
//! ```

use std::collections::hash_map::DefaultHasher;
use std::fs::{create_dir_all, read_to_string, write};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use tracing::debug;

/// Configuration for RPC fork mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// JSON-RPC endpoint URL.
    pub url: Option<String>,
    /// Block number to fork at.
    pub block: Option<u64>,
    /// Maximum retry attempts after transient failure.
    pub retries: u32,
    /// Initial retry backoff in milliseconds (doubles each attempt).
    pub backoff_ms: u64,
    /// Optional rate limit: maximum requests per second.
    pub rate_limit: Option<u64>,
    /// Request timeout in milliseconds for each RPC call.
    pub timeout_ms: u64,
    /// Chain ID used for cache key derivation and validation.
    pub chain_id: u64,
    /// Directory for the disk cache layer.
    pub cache_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    /// Create a new configuration with sensible defaults.
    pub fn new() -> Self {
        Self {
            url: None,
            block: None,
            retries: 3,
            backoff_ms: 100,
            rate_limit: None,
            timeout_ms: 30_000,
            chain_id: 1,
            cache_dir: None,
        }
    }

    /// Set the JSON-RPC endpoint URL.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the block number to fork at.
    pub fn block(mut self, block: u64) -> Self {
        self.block = Some(block);
        self
    }

    /// Set the maximum retry attempts after transient failure.
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// Set the initial retry backoff in milliseconds (doubles each attempt).
    pub fn backoff_ms(mut self, ms: u64) -> Self {
        self.backoff_ms = ms;
        self
    }

    /// Set an optional rate limit (maximum requests per second).
    pub fn rate_limit(mut self, limit: Option<u64>) -> Self {
        self.rate_limit = limit;
        self
    }

    /// Set the request timeout in milliseconds for each RPC call.
    pub fn timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Set the chain ID used for cache key derivation and validation.
    pub fn chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = chain_id;
        self
    }

    /// Set the directory for the disk cache layer.
    pub fn cache_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.cache_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    // -----------------------------------------------------------------
    // Validation
    // -----------------------------------------------------------------

    /// Validate the configuration.
    ///
    /// Ensures that:
    /// - a URL is configured,
    /// - a fork block number is set,
    /// - the URL reports the same `chain_id` as the configured one.
    pub fn validate(&self) -> Result<()> {
        let url = self.url.as_deref().context("RPC URL is required")?;
        ensure!(self.block.is_some(), "fork block number is required");

        let expected = self.chain_id;
        debug!(%url, "fetching chain_id from URL");
        let id = self.get_chain_id(url)?;
        ensure!(
            id == expected,
            "URL {url} reports chain_id {id}, expected {expected}"
        );

        debug!(chain_id = expected, "RPC config validated successfully");
        Ok(())
    }

    // -----------------------------------------------------------------
    // Chain ID
    // -----------------------------------------------------------------

    /// Query `eth_chainId` from a single RPC endpoint.
    ///
    /// Reads from `{cache_dir}/rpc/chain_id/{url_hash}` when a cache directory
    /// is configured and the file exists. Otherwise performs a plain HTTP POST
    /// with `ureq` (no pooling, deduplication, or rate limiting) and writes
    /// the result to disk when a cache directory is configured.
    pub fn get_chain_id(&self, url: &str) -> Result<u64> {
        if let Some(ref cache_dir) = self.cache_dir
            && let Some(id) = Self::read_cached_chain_id(cache_dir, url)?
        {
            return Ok(id);
        }

        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_chainId",
            "params": [],
            "id": 1
        });
        let body = serde_json::to_vec(&payload).context("serializing eth_chainId payload")?;

        let cfg = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(self.timeout_ms)))
            .build();
        let agent = ureq::Agent::new_with_config(cfg);

        let mut response = agent
            .post(url)
            .header("Content-Type", "application/json")
            .send(&body)
            .with_context(|| format!("sending eth_chainId request to {url}"))?;

        let text = response
            .body_mut()
            .read_to_string()
            .context("reading eth_chainId response body")?;

        let value: serde_json::Value =
            serde_json::from_str(&text).context("parsing eth_chainId response")?;

        let result = value
            .get("result")
            .and_then(|v| v.as_str())
            .with_context(|| format!("missing result in eth_chainId response from {url}"))?;

        let hex = result.strip_prefix("0x").unwrap_or(result);
        let chain_id = u64::from_str_radix(hex, 16)
            .with_context(|| format!("parsing chain_id hex {result} from {url}"))?;

        if let Some(ref cache_dir) = self.cache_dir {
            let _ = Self::write_cached_chain_id(cache_dir, url, chain_id);
        }

        Ok(chain_id)
    }

    fn read_cached_chain_id(cache_dir: impl AsRef<Path>, url: &str) -> Result<Option<u64>> {
        let cache_dir = cache_dir.as_ref();
        let url_hash = Self::url_hash(url);
        let cache_file = cache_dir.join("rpc").join("chain_id").join(url_hash);

        if !cache_file.exists() {
            return Ok(None);
        }

        let hex = read_to_string(&cache_file)
            .with_context(|| format!("reading chain_id cache file {}", cache_file.display()))?;
        let hex = hex.trim();
        let hex = hex.strip_prefix("0x").unwrap_or(hex);
        let chain_id = u64::from_str_radix(hex, 16)
            .with_context(|| format!("parsing cached chain_id from {}", cache_file.display()))?;

        Ok(Some(chain_id))
    }

    fn write_cached_chain_id(cache_dir: impl AsRef<Path>, url: &str, chain_id: u64) -> Result<()> {
        let cache_dir = cache_dir.as_ref();
        let url_hash = Self::url_hash(url);
        let cache_file = cache_dir.join("rpc").join("chain_id").join(&url_hash);

        let parent = cache_file
            .parent()
            .context("cache file has no parent directory")?;
        create_dir_all(parent).with_context(|| "creating chain_id cache directory")?;
        write(&cache_file, format!("0x{:x}", chain_id))
            .with_context(|| format!("writing chain_id cache file {}", cache_file.display()))?;

        Ok(())
    }

    fn url_hash(url: &str) -> String {
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir_all, write};

    use super::*;

    fn seed_chain_id_cache(cache_dir: impl AsRef<Path>, url: &str, chain_id: u64) {
        let cache_dir = cache_dir.as_ref();
        let url_hash = Config::url_hash(url);
        let dir = cache_dir.join("rpc").join("chain_id");
        create_dir_all(&dir).unwrap();
        write(dir.join(&url_hash), format!("0x{:x}", chain_id)).unwrap();
    }

    #[test]
    fn config_builder_roundtrip() {
        let config = Config::new()
            .url("http://localhost:8545")
            .block(1_000_000)
            .retries(5)
            .backoff_ms(200)
            .rate_limit(Some(10))
            .timeout_ms(5_000)
            .chain_id(1)
            .cache_dir("/tmp/cache");

        assert_eq!(config.url, Some("http://localhost:8545".to_string()));
        assert_eq!(config.block, Some(1_000_000));
        assert_eq!(config.retries, 5);
        assert_eq!(config.backoff_ms, 200);
        assert_eq!(config.rate_limit, Some(10));
        assert_eq!(config.timeout_ms, 5_000);
        assert_eq!(config.chain_id, 1);
        assert_eq!(
            config.cache_dir,
            Some(std::path::PathBuf::from("/tmp/cache"))
        );
    }

    #[test]
    fn config_defaults() {
        let config = Config::new();
        assert_eq!(config.url, None);
        assert_eq!(config.block, None);
        assert_eq!(config.retries, 3);
        assert_eq!(config.backoff_ms, 100);
        assert_eq!(config.rate_limit, None);
        assert_eq!(config.timeout_ms, 30_000);
        assert_eq!(config.chain_id, 1);
        assert_eq!(config.cache_dir, None);
    }

    // -----------------------------------------------------------------
    // validate
    // -----------------------------------------------------------------

    #[test]
    fn validate_fails_without_url() {
        let config = Config::new().block(1);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("RPC URL is required"));
    }

    #[test]
    fn validate_fails_without_block() {
        let config = Config::new().url("http://a.com");
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("fork block number is required"));
    }

    #[test]
    fn validate_succeeds_when_url_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new()
            .url("http://a.com")
            .block(1)
            .cache_dir(tmp.path());

        seed_chain_id_cache(tmp.path(), "http://a.com", 1);

        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_fails_on_chain_id_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new()
            .url("http://a.com")
            .block(1)
            .cache_dir(tmp.path());

        seed_chain_id_cache(tmp.path(), "http://a.com", 56);

        let err = config.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("56"));
        assert!(msg.contains("expected 1"));
    }

    // -----------------------------------------------------------------
    // get_chain_id
    // -----------------------------------------------------------------

    #[test]
    fn get_chain_id_reads_from_disk_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new()
            .url("http://dummy.example.com")
            .cache_dir(tmp.path());

        seed_chain_id_cache(tmp.path(), "http://dummy.example.com", 8453);

        let result = config.get_chain_id("http://dummy.example.com").unwrap();
        assert_eq!(result, 8453);
    }

    #[test]
    fn get_chain_id_returns_error_for_unreachable_url() {
        let config = Config::new().url("http://127.0.0.1:1").timeout_ms(100);

        let result = config.get_chain_id("http://127.0.0.1:1");
        assert!(result.is_err());
    }

    #[test]
    fn get_chain_id_caches_after_fetch() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::new()
            .url("http://127.0.0.1:1")
            .cache_dir(tmp.path())
            .timeout_ms(100);

        // First call fails because no cache and no server.
        let _ = config.get_chain_id("http://127.0.0.1:1");

        // Manually seed the cache so the second call succeeds.
        seed_chain_id_cache(tmp.path(), "http://127.0.0.1:1", 42);

        let result = config.get_chain_id("http://127.0.0.1:1").unwrap();
        assert_eq!(result, 42);
    }
}
