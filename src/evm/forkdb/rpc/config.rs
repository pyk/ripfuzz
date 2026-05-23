//! RPC client configuration.

use std::path::{Path, PathBuf};

/// Configuration for a single JSON-RPC endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// JSON-RPC endpoint URL.
    pub url: String,
    /// Maximum retry attempts after transient failure.
    pub retries: u32,
    /// Initial retry backoff in milliseconds (doubles each attempt).
    pub backoff_ms: u64,
    /// Optional rate limit: maximum requests per second.
    pub rate_limit: Option<u64>,
    /// Request timeout in milliseconds for each RPC call.
    pub timeout_ms: u64,
    /// Directory for the disk cache layer.
    pub cache_dir: Option<PathBuf>,
    /// Maximum number of JSON-RPC requests per HTTP batch.
    pub batch_size: usize,
    /// Maximum milliseconds to wait before flushing a partial batch.
    pub batch_timeout_ms: u64,
}

impl Config {
    /// Create a new configuration with sensible defaults.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            retries: 3,
            backoff_ms: 100,
            rate_limit: None,
            timeout_ms: 5_000,
            cache_dir: None,
            batch_size: 10,
            batch_timeout_ms: 5,
        }
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

    /// Set the directory for the disk cache layer.
    pub fn cache_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.cache_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Set the maximum number of JSON-RPC requests per HTTP batch.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Set the maximum milliseconds to wait before flushing a partial batch.
    pub fn batch_timeout_ms(mut self, ms: u64) -> Self {
        self.batch_timeout_ms = ms;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_builder_roundtrip() {
        let config = Config::new("http://localhost:8545")
            .retries(5)
            .backoff_ms(200)
            .rate_limit(Some(10))
            .timeout_ms(5_000)
            .cache_dir("/tmp/cache")
            .batch_size(20)
            .batch_timeout_ms(10);

        assert_eq!(config.url, "http://localhost:8545");
        assert_eq!(config.retries, 5);
        assert_eq!(config.backoff_ms, 200);
        assert_eq!(config.rate_limit, Some(10));
        assert_eq!(config.timeout_ms, 5_000);
        assert_eq!(
            config.cache_dir,
            Some(std::path::PathBuf::from("/tmp/cache"))
        );
        assert_eq!(config.batch_size, 20);
        assert_eq!(config.batch_timeout_ms, 10);
    }

    #[test]
    fn config_defaults() {
        let config = Config::new("http://localhost:8545");
        assert_eq!(config.url, "http://localhost:8545");
        assert_eq!(config.retries, 3);
        assert_eq!(config.backoff_ms, 100);
        assert_eq!(config.rate_limit, None);
        assert_eq!(config.timeout_ms, 5_000);
        assert_eq!(config.cache_dir, None);
        assert_eq!(config.batch_size, 10);
        assert_eq!(config.batch_timeout_ms, 5);
    }
}
