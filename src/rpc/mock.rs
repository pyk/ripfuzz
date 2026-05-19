//! Test helpers for the RPC module.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Context;

/// A mock RPC backend that returns pre-recorded responses.
#[derive(Debug, Clone, Default)]
pub struct FakeRpc {
    responses: Arc<Mutex<HashMap<(String, String), serde_json::Value>>>,
}

impl FakeRpc {
    /// Insert a canned response for a given method and serialized params.
    pub fn insert(
        &self,
        method: &str,
        params: &[serde_json::Value],
        response: serde_json::Value,
    ) -> anyhow::Result<()> {
        let params_json = serde_json::to_string(params).context("serialize params")?;
        let key = (method.into(), params_json);
        let mut guard = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(key, response);
        Ok(())
    }

    /// Call the mock, returning the pre-recorded response or an error.
    pub fn call(
        &self,
        method: &str,
        params: &[serde_json::Value],
    ) -> anyhow::Result<serde_json::Value> {
        let params_json = serde_json::to_string(params).context("serialize params")?;
        let key = (method.into(), params_json);
        let guard = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get(&key)
            .cloned()
            .context("FakeRpc: no response for {method} with {params:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_rpc_roundtrip() {
        let rpc = FakeRpc::default();
        rpc.insert("eth_blockNumber", &[], "0x1a2b".into()).unwrap();

        let result = rpc.call("eth_blockNumber", &[]).unwrap();
        assert_eq!(result, "0x1a2b");
    }

    #[test]
    fn fake_rpc_missing_response_errors() {
        let rpc = FakeRpc::default();
        let err = rpc.call("eth_blockNumber", &[]).unwrap_err();
        assert!(format!("{err}").contains("no response"));
    }
}
