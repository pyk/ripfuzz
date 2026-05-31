//! ForkDB: revm-native forked database with automatic batching,
//! two-layer caching, deduplication, rate limiting, and retries.
//!
//! Design goals:
//! 1. Strongly typed RPC requests and responses.
//! 2. Per-request disk cache (`{cache_dir}/eth_getStorageAt/{chain_id}/{block}/{addr}/{slot}.json`).
//! 3. Transparent translation between individual requests and JSON-RPC batches.
//! 4. Automatic batching via a shared backend that lets fuzzer threads
//!    themselves collect concurrent requests, group them, and dispatch
//!    responses back. No background worker thread.
//! 5. No fuzzer concepts leak in. Pure `DatabaseRef` implementation.

pub use backend::SharedBackend;
pub use config::ForkDBConfig;
pub use db::ForkDB;
pub use error::Error;
pub use request::{Request, url_hash};
pub use response::Response;
#[cfg_attr(not(test), allow(unused_imports))]
pub use transport::{MockTransport, Transport};

mod backend;
mod config;
mod db;
mod error;
mod limiter;
mod request;
mod response;
mod transport;
