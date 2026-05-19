//! RPC helpers for standalone operations (no connection pooling).

use std::collections::hash_map::DefaultHasher;
use std::fs::{create_dir_all, read_to_string, write};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

/// Query `eth_chainId` from an RPC endpoint and cache the result on disk.
///
/// The cache key is the hash of the URL string. This helper does not do
/// connection pooling, deduplication, or rate limiting.
pub fn get_chain_id(project_path: impl AsRef<Path>, rpc_url: &str) -> Result<u64> {
    let project_path = project_path.as_ref();
    let mut hasher = DefaultHasher::new();
    rpc_url.hash(&mut hasher);
    let url_hash = format!("{:x}", hasher.finish());

    let cache_dir = project_path.join("raptor").join("cache").join("chain_id");
    let cache_file = cache_dir.join(&url_hash);

    if cache_file.exists() {
        let hex = read_to_string(&cache_file)
            .with_context(|| format!("reading chain_id cache file {}", cache_file.display()))?;
        let hex = hex.trim();
        let hex = hex.strip_prefix("0x").unwrap_or(hex);
        return u64::from_str_radix(hex, 16)
            .with_context(|| format!("parsing cached chain_id from {}", cache_file.display()));
    }

    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_chainId",
        "params": [],
        "id": 1
    });

    let body = serde_json::to_vec(&payload).context("serializing eth_chainId payload")?;

    let cfg = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .build();
    let agent = ureq::Agent::new_with_config(cfg);
    let response = agent
        .post(rpc_url)
        .header("Content-Type", "application/json")
        .send(&body)
        .with_context(|| format!("sending eth_chainId request to {}", rpc_url))?;

    let mut response = response;
    let text = response
        .body_mut()
        .read_to_string()
        .context("reading eth_chainId response body")?;

    let value: serde_json::Value =
        serde_json::from_str(&text).context("parsing eth_chainId response")?;

    let result = value
        .get("result")
        .and_then(|v| v.as_str())
        .context("missing result in eth_chainId response")?;

    let hex = result.strip_prefix("0x").unwrap_or(result);
    let chain_id =
        u64::from_str_radix(hex, 16).with_context(|| format!("parsing chain_id hex {}", result))?;

    create_dir_all(&cache_dir)
        .with_context(|| format!("creating chain_id cache directory {}", cache_dir.display()))?;
    write(&cache_file, result)
        .with_context(|| format!("writing chain_id cache file {}", cache_file.display()))?;

    Ok(chain_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_chain_id_reads_from_disk_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let url = "https://dummy.example.com/rpc";

        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        let url_hash = format!("{:x}", hasher.finish());

        let cache_dir = tmp.path().join("raptor").join("cache").join("chain_id");
        create_dir_all(&cache_dir).unwrap();
        write(cache_dir.join(&url_hash), "0x2105").unwrap();

        let result = get_chain_id(tmp.path(), url).unwrap();
        assert_eq!(result, 8453);
    }
}
