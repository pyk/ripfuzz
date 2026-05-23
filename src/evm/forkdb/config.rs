//! Configuration for the RPC client and batching behaviour.

use std::path::{Path, PathBuf};

/// Configuration for the RPC client and batching behaviour.
#[derive(Debug, Clone)]
pub struct Config {
    pub url: String,
    pub retries: u32,
    pub backoff_ms: u64,
    pub timeout_ms: u64,
    pub batch_size: usize,
    pub batch_timeout_ms: u64,
    pub rate_limit: Option<u64>,
    pub cache_dir: Option<PathBuf>,
}

impl Config {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            retries: 3,
            backoff_ms: 100,
            timeout_ms: 30_000,
            batch_size: 50,
            batch_timeout_ms: 5,
            rate_limit: None,
            cache_dir: None,
        }
    }

    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    pub fn backoff_ms(mut self, ms: u64) -> Self {
        self.backoff_ms = ms;
        self
    }

    pub fn timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    pub fn cache_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.cache_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    pub fn rate_limit(mut self, rps: Option<u64>) -> Self {
        self.rate_limit = rps;
        self
    }

    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    pub fn batch_timeout_ms(mut self, ms: u64) -> Self {
        self.batch_timeout_ms = ms;
        self
    }
}
