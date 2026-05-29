//! Transaction type for executing call sequences.

use alloy_primitives::{Address, U256};
use revm::primitives::Bytes;

/// A single CALL transaction to execute in a sequence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Transaction {
    pub caller: Address,
    pub target: Address,
    pub calldata: Bytes,
    pub value: U256,
    pub gas_limit: u64,
}

impl Transaction {
    /// Create a [`Transaction`] for the given target.
    ///
    /// Caller defaults to [`DEFAULT_DEPLOYER`](super::DEFAULT_DEPLOYER); override with [`Self::caller`].
    /// Calldata defaults to empty bytes; override with [`Self::calldata`].
    pub fn new(target: Address) -> Self {
        Self {
            caller: super::DEFAULT_DEPLOYER,
            target,
            calldata: Bytes::new(),
            value: U256::ZERO,
            gas_limit: u64::MAX,
        }
    }

    /// Set the calldata for the transaction.
    pub fn calldata(mut self, calldata: Bytes) -> Self {
        self.calldata = calldata;
        self
    }

    /// Set the account address used to send the transaction.
    pub fn caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    /// Set the wei value sent with the transaction.
    pub fn value(mut self, value: U256) -> Self {
        self.value = value;
        self
    }

    /// Set the gas limit for the transaction.
    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self
    }
}
