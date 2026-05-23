//! ForkDB RPC client.
//!
//! Self-contained JSON-RPC client with:
//! - Two-layer caching (memory + structured disk)
//! - Per-RPC-method cache directories
//! - In-flight request deduplication
//! - Rate limiting
//! - Retries with exponential backoff
//! - Automatic request batching
//! - Mock transport for testing
//!
//! Public API exposes exactly four high-level methods:
//! - [`Client::get_remote_chain_info`]
//! - [`Client::get_remote_account_info`]
//! - [`Client::get_remote_block_info`]
//! - [`Client::get_remote_storage_info`]

pub use batcher::Batcher;
pub use cache::Cache;
pub use client::Client;
pub use config::Config;
pub use dedup::DedupTable;
pub use limiter::RateLimiter;
pub use transport::{MockTransport, Transport};
pub use types::{
    ChainIdResponse, GetBalanceResponse, GetBlockByNumberResponse, GetCodeResponse,
    GetStorageAtResponse, GetTransactionCountResponse, RemoteAccountInfo, RemoteBlockInfo,
    RemoteChainInfo, RpcRequest,
};

pub mod batcher;
pub mod cache;
pub mod client;
pub mod config;
pub mod dedup;
pub mod limiter;
pub mod transport;
pub mod types;
