//! Transport abstraction for JSON-RPC execution (live HTTP and mock).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use parking_lot::Mutex;

/// Trait for anything that can execute a JSON-RPC request.
///
/// The transport is intentionally dumb: it receives a JSON payload and a URL,
/// sends the payload, and returns the raw JSON response. No retries, no caching,
/// no deduplication, no rate limiting.
pub trait Transport: Send + Sync + std::fmt::Debug {
    fn exec(&self, url: &str, payload: &serde_json::Value) -> Result<serde_json::Value>;
}

impl Transport for ureq::Agent {
    fn exec(&self, url: &str, payload: &serde_json::Value) -> Result<serde_json::Value> {
        let body = serde_json::to_vec(payload).context("serializing RPC payload")?;
        let mut response = self
            .post(url)
            .header("Content-Type", "application/json")
            .send(&body)
            .context("sending RPC request")?;
        let text = response
            .body_mut()
            .read_to_string()
            .context("reading RPC response body")?;
        let value: serde_json::Value = serde_json::from_str(&text).context("json decode")?;
        Ok(value)
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Default, Clone)]
pub struct MockTransport {
    responses: Arc<Mutex<HashMap<(String, String), serde_json::Value>>>,
    delay: Arc<Mutex<Option<Duration>>>,
    call_count: Arc<Mutex<HashMap<(String, String), usize>>>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl MockTransport {
    /// Register a single mock response for a given URL and serialized payload.
    pub fn mock_response(
        &self,
        url: &str,
        payload: &serde_json::Value,
        response: serde_json::Value,
    ) {
        let payload_json = serde_json::to_string(payload).unwrap_or_default();
        let mut guard = self.responses.lock();
        guard.insert((url.into(), payload_json), response);
    }

    /// Return how many times a given request was dispatched.
    pub fn call_count(&self, url: &str, payload: &serde_json::Value) -> usize {
        let payload_json = serde_json::to_string(payload).unwrap_or_default();
        let guard = self.call_count.lock();
        guard.get(&(url.into(), payload_json)).copied().unwrap_or(0)
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl Transport for MockTransport {
    fn exec(&self, url: &str, payload: &serde_json::Value) -> Result<serde_json::Value> {
        if let Some(delay) = *self.delay.lock() {
            std::thread::sleep(delay);
        }

        let payload_json = serde_json::to_string(payload).unwrap_or_default();
        let key = (url.into(), payload_json);

        {
            let mut guard = self.call_count.lock();
            let count = guard.entry(key.clone()).or_insert(0);
            *count += 1;
        }

        let response = {
            let guard = self.responses.lock();
            guard.get(&key).cloned()
        }
        .with_context(|| format!("MockTransport: no response for url={url} payload={payload}"))?;

        Ok(response)
    }
}
