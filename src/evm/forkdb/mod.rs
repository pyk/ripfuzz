//! ForkDB: revm-native forked database with automatic batching,
//! two-layer caching, deduplication, rate limiting, and retries.
//!
//! Design goals:
//! 1. Strongly typed RPC requests and responses.
//! 2. Per-request disk cache (`{cache_dir}/eth_getStorageAt/{block}/{addr}/{slot}.json`).
//! 3. Transparent translation between individual requests and JSON-RPC batches.
//! 4. Automatic batching via a background worker that collects concurrent
//!    requests, groups them, and dispatches responses back.
//! 5. No fuzzer concepts leak in. Pure `DatabaseRef` implementation.

pub use cache::Cache;
pub use client::Client;
pub use config::Config;
pub use db::ForkDB;
pub use error::Error;
pub use request::{Request, url_hash};
pub use response::{Block, Response};
pub use transport::{MockTransport, Transport};

mod batcher;
mod cache;
mod client;
mod config;
mod db;
mod dedup;
mod error;
mod limiter;
mod request;
mod response;
mod transport;
