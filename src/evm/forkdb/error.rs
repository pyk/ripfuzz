//! Enumerated error type for ForkDB so callers can programmatically
//! distinguish between transient and permanent failures.

use revm::database_interface::DBErrorMarker;

/// Error type for ForkDB operations.
#[derive(Debug, Clone)]
pub enum Error {
    /// RPC transport timed out (e.g., HTTP request exceeded deadline).
    RpcTimeout { url: String },
    /// Rate limited by the RPC provider (HTTP 429 or similar).
    RateLimited { url: String },
    /// Failed to serialize or deserialize JSON.
    DecodeError { message: String },
    /// The RPC server returned a JSON-RPC error object.
    RpcError { code: i64, message: String },
    /// An unexpected response was received (duplicate, missing, wrong variant).
    UnexpectedResponse { message: String },
    /// The requested account or code hash is not present in the fork database.
    MissingAccount { message: String },
    /// The batcher worker restarted (e.g. after a panic). The request should
    /// be retried by higher-level logic.
    BatcherRestarted,
    /// An internal error (channel closed, worker shut down, etc.).
    Internal { message: String },
}

impl Error {
    /// Returns `true` if this error is likely transient and the request
    /// should be retried.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::RpcTimeout { .. } | Self::RateLimited { .. } | Self::BatcherRestarted
        )
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RpcTimeout { url } => write!(f, "RPC timeout: {url}"),
            Self::RateLimited { url } => write!(f, "RPC rate limited: {url}"),
            Self::DecodeError { message } => write!(f, "RPC decode error: {message}"),
            Self::RpcError { code, message } => write!(f, "RPC error {code}: {message}"),
            Self::UnexpectedResponse { message } => {
                write!(f, "unexpected RPC response: {message}")
            }
            Self::MissingAccount { message } => {
                write!(f, "missing account in fork DB: {message}")
            }
            Self::BatcherRestarted => {
                write!(f, "batcher restarted, request should be retried")
            }
            Self::Internal { message } => write!(f, "internal fork DB error: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl DBErrorMarker for Error {}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        let msg = format!("{e}");
        // Classify transport errors by inspecting the root cause.
        if let Some(ureq_err) = e.root_cause().downcast_ref::<ureq::Error>() {
            match ureq_err {
                ureq::Error::StatusCode(429) => {
                    return Self::RateLimited { url: String::new() };
                }
                ureq::Error::Timeout(_) => {
                    return Self::RpcTimeout { url: String::new() };
                }
                _ => {}
            }
        }
        if let Some(json_err) = e.root_cause().downcast_ref::<serde_json::Error>() {
            return Self::DecodeError {
                message: format!("{json_err}"),
            };
        }
        if msg.contains("429") || msg.contains("rate limit") || msg.contains("too many requests") {
            return Self::RateLimited { url: String::new() };
        }
        if msg.contains("timeout") || msg.contains("timed out") {
            return Self::RpcTimeout { url: String::new() };
        }
        if msg.contains("batcher restarted") {
            return Self::BatcherRestarted;
        }
        Self::Internal { message: msg }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: Error must be an enumerated type so callers can
    /// programmatically distinguish between transient and permanent failures.
    #[test]
    fn error_variants_are_programmatically_distinguishable() {
        let timeout = Error::RpcTimeout {
            url: "http://rpc.example".into(),
        };
        assert!(matches!(timeout, Error::RpcTimeout { .. }));
        assert!(timeout.is_transient());

        let rate_limited = Error::RateLimited {
            url: "http://rpc.example".into(),
        };
        assert!(matches!(rate_limited, Error::RateLimited { .. }));
        assert!(rate_limited.is_transient());

        let rpc_err = Error::RpcError {
            code: -32000,
            message: "bad block".into(),
        };
        assert!(matches!(rpc_err, Error::RpcError { .. }));
        assert!(!rpc_err.is_transient());

        let decode = Error::DecodeError {
            message: "invalid json".into(),
        };
        assert!(matches!(decode, Error::DecodeError { .. }));
        assert!(!decode.is_transient());

        let restarted = Error::BatcherRestarted;
        assert!(matches!(restarted, Error::BatcherRestarted));
        assert!(restarted.is_transient());

        let from_anyhow = Error::from(anyhow::anyhow!("batcher restarted"));
        assert!(
            matches!(from_anyhow, Error::BatcherRestarted),
            "anyhow containing 'batcher restarted' must map to Error::BatcherRestarted"
        );
        assert!(from_anyhow.is_transient());
    }
}
