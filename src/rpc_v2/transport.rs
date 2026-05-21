//! Transport abstraction: HTTP and mock implementations.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};

use crate::rpc_v2::client::{AgentPool, UrlPool};
use crate::rpc_v2::request;

/// Trait for anything that can execute a JSON-RPC request.
pub trait Transport: Send + Sync + std::fmt::Debug {
    /// Send a JSON-RPC request and return the `result` field.
    fn send(&self, payload: serde_json::Value) -> Result<serde_json::Value>;

    /// Query every configured endpoint for `eth_chainId` and ensure they agree.
    fn validate_chain_id(&self) -> Result<u64>;
}

// ----------------------------------------------------------------------------
// HTTP Transport
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct HttpTransport {
    urls: UrlPool,
    agents: AgentPool,
    retries: u32,
    retry_backoff: Duration,
}

impl HttpTransport {
    pub fn new(
        urls: Vec<String>,
        agents: Vec<ureq::Agent>,
        retries: u32,
        retry_backoff: Duration,
    ) -> Self {
        Self {
            urls: UrlPool::new(urls),
            agents: AgentPool::new(agents),
            retries,
            retry_backoff,
        }
    }
}

impl Transport for HttpTransport {
    fn send(&self, payload: serde_json::Value) -> Result<serde_json::Value> {
        let body = serde_json::to_vec(&payload).context("serializing RPC payload")?;
        let url_count = self.urls.urls().len();
        let mut last_err: Option<anyhow::Error> = None;

        for _url_idx in 0..url_count {
            let url = self.urls.next();
            for attempt in 0..=self.retries {
                let agent = self.agents.next();
                match agent
                    .post(url)
                    .header("Content-Type", "application/json")
                    .send(&body)
                {
                    Ok(mut response) => {
                        let text = response
                            .body_mut()
                            .read_to_string()
                            .context("reading RPC response body")?;
                        let value: serde_json::Value =
                            serde_json::from_str(&text).context("json decode")?;

                        match value {
                            serde_json::Value::Object(ref map) if map.get("error").is_some() => {
                                let err = &map["error"];
                                last_err = Some(anyhow::anyhow!("RPC error: {err}"));
                            }
                            serde_json::Value::Object(mut map) => {
                                if let Some(result) = map.remove("result") {
                                    return Ok(result);
                                }
                                last_err = Some(anyhow::anyhow!("missing result field"));
                            }
                            _ => {
                                last_err = Some(anyhow::anyhow!("missing result field"));
                            }
                        }
                    }
                    Err(e) => {
                        last_err = Some(anyhow::Error::new(e));
                    }
                }

                if attempt < self.retries {
                    std::thread::sleep(self.retry_backoff * (attempt + 1));
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("RPC request failed on all URLs")))
    }

    fn validate_chain_id(&self) -> Result<u64> {
        let payload = request::payload("eth_chainId", &[]);
        let body = serde_json::to_vec(&payload).context("serializing eth_chainId payload")?;
        let mut ids: Vec<u64> = Vec::new();

        for url in self.urls.urls() {
            let agent = self.agents.next();
            let mut response = agent
                .post(url)
                .header("Content-Type", "application/json")
                .send(&body)
                .with_context(|| format!("sending eth_chainId request to {}", url))?;
            let text = response
                .body_mut()
                .read_to_string()
                .context("reading eth_chainId response body")?;
            let value: serde_json::Value =
                serde_json::from_str(&text).context("parsing eth_chainId response")?;

            let result = value
                .get("result")
                .and_then(|v| v.as_str())
                .with_context(|| format!("missing result in eth_chainId response from {}", url))?;
            let hex = result.strip_prefix("0x").unwrap_or(result);
            let chain_id = u64::from_str_radix(hex, 16)
                .with_context(|| format!("parsing chain_id hex {result} from {}", url))?;
            ids.push(chain_id);
        }

        ensure!(!ids.is_empty(), "no RPC URLs to validate");

        let first = ids[0];
        ensure!(
            ids.iter().all(|id| *id == first),
            "chain ID mismatch: {:?}",
            ids
        );

        Ok(first)
    }
}

// ----------------------------------------------------------------------------
// Mock Transport
// ----------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct MockTransport {
    responses: Arc<Mutex<HashMap<(String, String), serde_json::Value>>>,
    chain_id: Arc<Mutex<Option<u64>>>,
    delay: Arc<Mutex<Option<Duration>>>,
    call_count: Arc<Mutex<HashMap<(String, String), usize>>>,
}

impl MockTransport {
    /// Insert a canned response for a given method and serialized params.
    pub fn insert(&self, method: &str, params: &[serde_json::Value], response: serde_json::Value) {
        let params_json = serde_json::to_string(params).unwrap_or_default();
        let mut guard = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert((method.into(), params_json), response);
    }

    /// Set the chain_id returned by `validate_chain_id`.
    pub fn set_chain_id(&self, chain_id: u64) {
        *self.chain_id.lock().unwrap_or_else(|e| e.into_inner()) = Some(chain_id);
    }

    /// Set an artificial delay for every `send` call.
    pub fn set_delay(&self, delay: Duration) {
        *self.delay.lock().unwrap_or_else(|e| e.into_inner()) = Some(delay);
    }

    /// Return how many times a given request was dispatched.
    pub fn call_count(&self, method: &str, params: &[serde_json::Value]) -> usize {
        let params_json = serde_json::to_string(params).unwrap_or_default();
        let guard = self.call_count.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get(&(method.into(), params_json))
            .copied()
            .unwrap_or(0)
    }
}

impl Transport for MockTransport {
    fn send(&self, payload: serde_json::Value) -> Result<serde_json::Value> {
        if let Some(delay) = *self.delay.lock().unwrap_or_else(|e| e.into_inner()) {
            std::thread::sleep(delay);
        }

        let method = payload.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let params = payload.get("params").cloned().unwrap_or_default();
        let params_json = serde_json::to_string(&params).unwrap_or_default();
        let key = (method.into(), params_json.clone());

        {
            let mut guard = self.call_count.lock().unwrap_or_else(|e| e.into_inner());
            *guard.entry(key.clone()).or_insert(0) += 1;
        }

        let guard = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get(&key)
            .cloned()
            .with_context(|| format!("MockTransport: no response for {method} with {params:?}"))
    }

    fn validate_chain_id(&self) -> Result<u64> {
        let guard = self.chain_id.lock().unwrap_or_else(|e| e.into_inner());
        (*guard).context("MockTransport: chain_id not set")
    }
}
