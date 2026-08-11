//! A single `max_*` function whose return value is maximized.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};

use crate::corpus::Call;
use crate::evm::{Transaction, TransactionResult};

/// A read-only harness function whose `uint256` return value is maximized.
#[derive(Debug, Clone)]
pub struct MaxObjective {
    pub function: Function,
}

impl MaxObjective {
    /// Create a max objective from its harness function.
    pub fn new(function: Function) -> Self {
        Self { function }
    }

    /// Build the no-argument call used to evaluate this objective.
    pub fn call(&self, caller: Address) -> Call {
        Call {
            function: self.function.clone(),
            args: DynSolValue::Tuple(vec![]),
            value: None,
            caller,
        }
    }

    /// Build the transaction used to evaluate this objective.
    pub fn transaction(&self, target: Address, caller: Address, gas_limit: u64) -> Transaction {
        self.call(caller)
            .into_transaction(target)
            .gas_limit(gas_limit)
    }

    /// Decode a successful `uint256` return value.
    ///
    /// Reverted or empty results decode to `None`, which callers treat as the
    /// minimum score (`0`).
    pub fn decode(&self, result: &TransactionResult) -> Option<U256> {
        if !result.success {
            return None;
        }
        let output = result.output.as_ref()?;
        let value = DynSolType::Uint(256).abi_decode(output).ok()?;
        match value {
            DynSolValue::Uint(value, 256) => Some(value),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_json_abi::Function;
    use alloy_primitives::U256;
    use revm::primitives::Bytes;

    use super::*;
    use crate::evm::TransactionResult;

    fn objective() -> MaxObjective {
        MaxObjective::new(Function::parse("max_value()").unwrap())
    }

    #[test]
    fn decode_success_uint256() {
        let mut output = vec![0u8; 32];
        output[31] = 42;
        let result = TransactionResult {
            success: true,
            gas_used: 0,
            output: Some(Bytes::from(output)),
            logs: vec![],
            created_address: None,
        };

        assert_eq!(objective().decode(&result), Some(U256::from(42)));
    }

    #[test]
    fn decode_revert_is_none() {
        let result = TransactionResult {
            success: false,
            gas_used: 0,
            output: Some(Bytes::from(vec![0u8; 32])),
            logs: vec![],
            created_address: None,
        };

        assert_eq!(objective().decode(&result), None);
    }

    #[test]
    fn decode_empty_output_is_none() {
        let result = TransactionResult {
            success: true,
            gas_used: 0,
            output: Some(Bytes::new()),
            logs: vec![],
            created_address: None,
        };

        assert_eq!(objective().decode(&result), None);
    }
}
