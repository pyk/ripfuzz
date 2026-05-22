//! RPC v2: self-contained JSON-RPC client with caching, deduplication,
//! rate limiting, retries, and typed EVM method wrappers.

pub use cache::Cache;
pub use client::{Block, Client};
pub use config::Config;
pub use dedup::DedupTable;
pub use limiter::RateLimiter;
pub use transport::{MockTransport, Transport};

mod cache;
mod client;
mod config;
mod dedup;
mod limiter;
mod request;
pub mod transport;
