//! Transport abstraction: HTTP and mock implementations.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::rpc_v2::client::{AgentPool, UrlPool};

/// Trait for anything that can execute a JSON-RPC request.
pub trait Transport: Send + Sync + std::fmt::Debug {
    /// Send a JSON-RPC request and return the full response envelope.
    fn send(&self, payload: serde_json::Value) -> Result<serde_json::Value>;

    /// Send a JSON-RPC batch request and return the full response envelopes.
    ///
    /// The default implementation loops over [`Self::send`] one request at a
    /// time.  Concrete transports that support true HTTP batching should
    /// override this.
    fn send_batch(&self, payloads: Vec<serde_json::Value>) -> Result<Vec<serde_json::Value>> {
        payloads.into_iter().map(|p| self.send(p)).collect()
    }
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
                            serde_json::Value::Object(_) => {
                                return Ok(value);
                            }
                            _ => {
                                last_err = Some(anyhow::anyhow!("invalid RPC response"));
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

    fn send_batch(&self, payloads: Vec<serde_json::Value>) -> Result<Vec<serde_json::Value>> {
        let body = serde_json::to_vec(&payloads).context("serializing RPC batch payload")?;
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
                            .context("reading RPC batch response body")?;
                        let value: serde_json::Value =
                            serde_json::from_str(&text).context("json decode")?;

                        match value {
                            serde_json::Value::Array(arr) => {
                                return Ok(arr);
                            }
                            serde_json::Value::Object(ref map) if map.get("error").is_some() => {
                                let err = &map["error"];
                                last_err = Some(anyhow::anyhow!("RPC batch error: {err}"));
                            }
                            _ => {
                                last_err = Some(anyhow::anyhow!("invalid RPC batch response"));
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

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("RPC batch request failed on all URLs")))
    }
}

// ----------------------------------------------------------------------------
// Mock Transport
// ----------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct MockTransport {
    responses: Arc<Mutex<HashMap<(String, String), serde_json::Value>>>,
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
}
