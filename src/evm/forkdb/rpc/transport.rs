//! Transport abstraction: HTTP and Mock implementations.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};

/// Trait for anything that can execute a JSON-RPC request.
///
/// The transport is intentionally dumb: it receives a JSON payload and a URL,
/// sends the payload, and returns the raw JSON response. No retries, no URL
/// rotation, no caching, no deduplication, no rate limiting.
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

#[derive(Debug, Default, Clone)]
#[allow(clippy::type_complexity)]
pub struct MockTransport {
    responses: Arc<Mutex<HashMap<(String, String), serde_json::Value>>>,
    sequences: Arc<Mutex<HashMap<(String, String), Vec<serde_json::Value>>>>,
    delay: Arc<Mutex<Option<Duration>>>,
    call_count: Arc<Mutex<HashMap<(String, String), usize>>>,
}

impl MockTransport {
    /// Register a single mock response for a given URL and serialized payload.
    pub fn mock_response(
        &self,
        url: &str,
        payload: &serde_json::Value,
        response: serde_json::Value,
    ) {
        let payload_json = serde_json::to_string(payload).unwrap_or_default();
        let mut guard = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert((url.into(), payload_json), response);
    }

    /// Register multiple mock responses for a given URL and serialized payload.
    /// On successive calls with the same key, responses are returned in order;
    /// the last response repeats once the list is exhausted.
    pub fn mock_responses(
        &self,
        url: &str,
        payload: &serde_json::Value,
        responses: Vec<serde_json::Value>,
    ) {
        let payload_json = serde_json::to_string(payload).unwrap_or_default();
        let mut guard = self.sequences.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert((url.into(), payload_json), responses);
    }

    /// Set an artificial delay for every `exec` call.
    pub fn set_delay(&self, delay: Duration) {
        *self.delay.lock().unwrap_or_else(|e| e.into_inner()) = Some(delay);
    }

    /// Return how many times a given request was dispatched.
    pub fn call_count(&self, url: &str, payload: &serde_json::Value) -> usize {
        let payload_json = serde_json::to_string(payload).unwrap_or_default();
        let guard = self.call_count.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(&(url.into(), payload_json)).copied().unwrap_or(0)
    }
}

impl Transport for MockTransport {
    fn exec(&self, url: &str, payload: &serde_json::Value) -> Result<serde_json::Value> {
        if let Some(delay) = *self.delay.lock().unwrap_or_else(|e| e.into_inner()) {
            std::thread::sleep(delay);
        }

        let payload_json = serde_json::to_string(payload).unwrap_or_default();
        let key = (url.into(), payload_json);

        let call_count = {
            let mut guard = self.call_count.lock().unwrap_or_else(|e| e.into_inner());
            let count = guard.entry(key.clone()).or_insert(0);
            *count += 1;
            *count
        };

        let mut response = {
            let guard = self.sequences.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(seq) = guard.get(&key) {
                let idx = (call_count - 1).min(seq.len().saturating_sub(1));
                Some(seq[idx].clone())
            } else {
                None
            }
        }
        .or_else(|| {
            let guard = self.responses.lock().unwrap_or_else(|e| e.into_inner());
            guard.get(&key).cloned()
        })
        .with_context(|| format!("MockTransport: no response for url={url} payload={payload}"))?;

        // Echo back the request id so callers that match on id work correctly.
        let id = payload.get("id").cloned();
        if let Some(id) = id
            && let Some(obj) = response.as_object_mut()
        {
            obj.insert("id".into(), id);
        }

        Ok(response)
    }
}
