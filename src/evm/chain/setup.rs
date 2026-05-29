//! Setup input and output types.

use alloy_primitives::{Address, U256};
use alloy_sol_types::SolCall;
use revm::primitives::Bytes;

use crate::evm::{result, trace};

/// Result of a setup call, including the trace.
#[derive(Debug, Clone)]
pub struct SetupOutput {
    pub result: result::TransactionResult,
    pub trace: trace::Trace,
}

alloy_sol_types::sol! {
    interface Setup {
        function setup() external;
    }
}

/// Configuration for a setup call.
#[derive(Debug, Clone)]
pub struct SetupInput {
    pub caller: Address,
    pub target: Address,
    pub calldata: Bytes,
    pub value: U256,
    pub gas_limit: u64,
}

impl SetupInput {
    /// Create [`SetupInput`] for the given target with the default `setup()` selector.
    ///
    /// Caller defaults to [`DEFAULT_DEPLOYER`](super::DEFAULT_DEPLOYER); override with [`Self::caller`].
    pub fn new(target: Address) -> Self {
        Self {
            caller: super::DEFAULT_DEPLOYER,
            target,
            calldata: Bytes::from(Setup::setupCall::new(()).abi_encode()),
            value: U256::ZERO,
            gas_limit: u64::MAX,
        }
    }

    /// Set the calldata for the setup transaction.
    pub fn calldata(mut self, calldata: Bytes) -> Self {
        self.calldata = calldata;
        self
    }

    /// Set the account address used to send the setup transaction.
    pub fn caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    /// Set the wei value sent with the setup transaction.
    pub fn value(mut self, value: U256) -> Self {
        self.value = value;
        self
    }

    /// Set the gas limit for the setup transaction.
    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self
    }
}
