//! RPC v2: self-contained JSON-RPC client with 2-layer caching, deduplication,
//! rate limiting, retries, and typed EVM method wrappers.

pub use cache::Cache;
pub use client::{AgentPool, Block, Client, UrlPool};
pub use config::Config;
pub use dedup::{DedupTable, RequestKey};
pub use limiter::RateLimiter;
pub use transport::{HttpTransport, MockTransport, Transport};

mod cache;
mod client;
mod config;
mod dedup;
mod limiter;
mod request;
pub mod transport;
