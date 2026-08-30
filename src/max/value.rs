//! Measured harness values.
//!
//! [`Value`] is the `uint256` reported by the harness `value` function,
//! decoded from a transaction result.
//!
//! ```rust
//! use ripfuzz::max::Value;
//!
//! // let result = chain.call(caller, address, U256::ZERO, calldata)?;
//! // let initial_value = Value::decode(&result)?;
//! ```

use std::fmt;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::U256;
use anyhow::{Context, Result, bail, ensure};

use crate::evm::TransactionResult;

/// The `uint256` value reported by a harness `value` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Value(U256);

impl Value {
    /// Create a value from a raw `uint256`.
    pub fn new(value: U256) -> Self {
        Self(value)
    }

    /// Decode a successful `uint256` call result.
    pub fn decode(result: &TransactionResult) -> Result<Self> {
        ensure!(result.success, "value call reverted");
        let output = result
            .output
            .as_ref()
            .context("value call returned no output")?;
        let decoded = DynSolType::Uint(256)
            .abi_decode(output)
            .context("value call output is not `uint256`")?;
        let DynSolValue::Uint(value, 256) = decoded else {
            bail!("value call output is not `uint256`");
        };
        Ok(Self(value))
    }

    /// The underlying `uint256` value.
    pub fn get(self) -> U256 {
        self.0
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use revm::primitives::Bytes;

    use super::*;

    fn result(success: bool, output: Option<Bytes>) -> TransactionResult {
        TransactionResult {
            success,
            gas_used: 0,
            output,
            ..Default::default()
        }
    }

    #[test]
    fn decodes_success_uint256() {
        let mut output = vec![0u8; 32];
        output[31] = 42;
        let result = result(true, Some(Bytes::from(output)));

        assert_eq!(Value::decode(&result).unwrap().get(), U256::from(42));
    }

    #[test]
    fn decode_revert_fails() {
        let result = result(false, Some(Bytes::from(vec![0u8; 32])));

        let err = Value::decode(&result).unwrap_err();
        assert_eq!(err.to_string(), "value call reverted");
    }

    #[test]
    fn decode_missing_output_fails() {
        let result = result(true, None);

        let err = Value::decode(&result).unwrap_err();
        assert_eq!(err.to_string(), "value call returned no output");
    }

    #[test]
    fn decode_short_output_fails() {
        let result = result(true, Some(Bytes::from(vec![0u8; 4])));

        let err = Value::decode(&result).unwrap_err();
        assert_eq!(err.to_string(), "value call output is not `uint256`");
    }

    #[test]
    fn decode_display_is_decimal() {
        let mut output = vec![0u8; 32];
        output[31] = 7;
        let value = Value::decode(&result(true, Some(Bytes::from(output)))).unwrap();

        assert_eq!(value.to_string(), "7");
    }
}
