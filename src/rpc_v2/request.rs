//! JSON-RPC payload builder and response helpers.

use serde_json::json;

/// Build a standard JSON-RPC 2.0 request payload.
pub fn payload(method: &str, params: &[serde_json::Value]) -> serde_json::Value {
    payload_with_id(method, params, 1)
}

/// Build a JSON-RPC 2.0 request payload with a custom request id.
pub fn payload_with_id(
    method: &str,
    params: &[serde_json::Value],
    id: impl Into<serde_json::Value>,
) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
        "method": method,
        "params": params,
    })
}

#[cfg(test)]
mod tests {
    use super::payload;

    #[test]
    fn payload_structure() {
        let p = payload("eth_blockNumber", &[]);
        assert_eq!(p["jsonrpc"], "2.0");
        assert_eq!(p["id"], 1);
        assert_eq!(p["method"], "eth_blockNumber");
        assert_eq!(p["params"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn payload_with_params() {
        let p = payload("eth_getBalance", &["0x0".into(), "latest".into()]);
        let arr = p["params"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], "0x0");
        assert_eq!(arr[1], "latest");
    }
}
