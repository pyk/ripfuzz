//! RPC client with automatic batching, caching, deduplication, rate limiting,
//! and retries.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use crossbeam::channel::{self, Sender};
use tracing::{instrument, trace};

use crate::evm::forkdb::batcher::{Batcher, PendingRequest};
use crate::evm::forkdb::cache::Cache;
use crate::evm::forkdb::config::Config;
use crate::evm::forkdb::dedup::DedupTable;
use crate::evm::forkdb::limiter::RateLimiter;
use crate::evm::forkdb::request::Request;
use crate::evm::forkdb::response::Response;
use crate::evm::forkdb::transport::Transport;

/// RPC client with automatic batching, two-layer caching, deduplication,
/// rate limiting, and retries.
///
/// Cloning is cheap (shares the same background worker and caches).
#[derive(Debug, Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

#[derive(Debug)]
struct ClientInner {
    request_tx: Sender<PendingRequest>,
    cache: Option<Arc<Cache>>,
    dedup: Arc<DedupTable>,
}

impl Client {
    /// Create a client with the default HTTP transport (`ureq`).
    pub fn new(config: Config) -> Self {
        let agent_cfg = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(config.timeout_ms)))
            .build();
        let agent = ureq::Agent::new_with_config(agent_cfg);
        Self::new_with_transport(config, agent)
    }

    /// Create a client with a custom transport (e.g. [`MockTransport`] for
    /// testing).
    pub fn new_with_transport(config: Config, transport: impl Transport + 'static) -> Self {
        let limiter = config.rate_limit.map(|r| Arc::new(RateLimiter::new(r)));
        let cache = config.cache_dir.map(|d| Arc::new(Cache::new(d)));
        let dedup = Arc::new(DedupTable::new());

        let (request_tx, request_rx) = channel::unbounded();

        let batcher = Batcher {
            request_rx,
            transport: Box::new(transport),
            url: config.url,
            retries: config.retries,
            backoff: Duration::from_millis(config.backoff_ms),
            batch_size: config.batch_size,
            batch_timeout: Duration::from_millis(config.batch_timeout_ms),
            cache: cache.clone(),
            dedup: dedup.clone(),
            limiter: limiter.clone(),
        };

        std::thread::spawn(move || batcher.run());

        Self {
            inner: Arc::new(ClientInner {
                request_tx,
                cache,
                dedup,
            }),
        }
    }

    /// Send one or more requests. They are automatically batched, deduplicated,
    /// rate-limited, retried, and cached.
    ///
    /// Even a single request goes through the batching worker so it benefits
    /// from the same caching, dedup, and retry logic as a full batch.
    #[instrument(skip(self), fields(count = reqs.len()))]
    pub fn request(&self, reqs: &[Request]) -> Result<Vec<Response>> {
        if reqs.is_empty() {
            return Ok(Vec::new());
        }

        let mut results: Vec<Option<Response>> = vec![None; reqs.len()];
        let mut to_fetch: Vec<(usize, String, crate::evm::forkdb::dedup::DedupGuard)> = Vec::new();

        // 1. Check cache and dedup for each request
        for (idx, req) in reqs.iter().enumerate() {
            if let Some(cache) = self.inner.cache.as_ref()
                && let Some(value) = cache.get(req)
            {
                match Response::parse(req, &value) {
                    Ok(resp) => {
                        results[idx] = Some(resp);
                        continue;
                    }
                    Err(e) => trace!(?e, "cached parse failed, refetching"),
                }
            }

            let cache_key = req.cache_key();
            if let Some(result) = self.inner.dedup.register(&cache_key) {
                trace!("dedup hit for {}", cache_key);
                if let Ok(resp) = result.and_then(|v| Response::parse(req, &v)) {
                    results[idx] = Some(resp);
                    continue;
                }
            }

            let guard = self.inner.dedup.guard(&cache_key);
            to_fetch.push((idx, cache_key, guard));
        }

        if to_fetch.is_empty() {
            return results
                .into_iter()
                .map(|o| o.context("missing response"))
                .collect::<Result<Vec<Response>>>();
        }

        // 2. Send all uncached / undeduped requests to the batching worker
        let mut receivers = Vec::with_capacity(to_fetch.len());
        for (idx, cache_key, guard) in to_fetch {
            let (tx, rx) = channel::bounded(1);
            self.inner
                .request_tx
                .send(PendingRequest {
                    request: reqs[idx].to_owned(),
                    response_tx: tx,
                })
                .map_err(|_| anyhow!("batch worker shut down"))?;
            receivers.push((idx, cache_key, guard, rx));
        }

        // 3. Collect all responses without failing early so every dedup
        //    guard gets properly completed even if one request errors.
        let mut resp_results = Vec::with_capacity(receivers.len());
        for (_, _, _, rx) in &receivers {
            resp_results.push(
                rx.recv()
                    .map_err(|_| anyhow!("batch worker response channel closed"))?,
            );
        }

        // 4. Cache, complete dedup, and fill results
        let mut any_err = None;
        for ((idx, cache_key, guard, _), resp_result) in receivers.into_iter().zip(resp_results) {
            let req = &reqs[idx];
            match resp_result {
                Ok(response) => {
                    if let Some(cache) = self.inner.cache.as_ref() {
                        cache.insert(req, response.to_json());
                    }
                    self.inner
                        .dedup
                        .complete(&cache_key, Ok(response.to_json()));
                    guard.deactivate();
                    results[idx] = Some(response);
                }
                Err(e) => {
                    self.inner.dedup.complete(&cache_key, Err(anyhow!("{e}")));
                    guard.deactivate();
                    any_err = Some(e);
                }
            }
        }

        if let Some(e) = any_err {
            return Err(e);
        }

        results
            .into_iter()
            .map(|o| o.context("missing response"))
            .collect::<Result<Vec<Response>>>()
    }
}
