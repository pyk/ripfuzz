//! Deployment input and output types.

use alloy_primitives::{Address, U256};

use crate::evm::chain::DEFAULT_DEPLOYER;
use crate::evm::{ExecutionCoverage, result, trace};

/// Configuration for a contract deployment.
#[derive(Debug, Clone)]
pub struct DeployInput {
    pub caller: Address,
    pub value: U256,
    pub initcode: String,
    pub libraries: Vec<DeployLibraryInput>,
    pub gas_limit: u64,
}

/// Configuration for a linked library deployment.
#[derive(Debug, Clone)]
pub struct DeployLibraryInput {
    pub id: String,
    pub initcode: String,
    pub libraries: Vec<DeployLibraryInput>,
}

impl DeployLibraryInput {
    /// Create [`DeployLibraryInput`] with the given identifier and initcode.
    pub fn new(id: impl Into<String>, initcode: &str) -> Self {
        Self {
            id: id.into(),
            initcode: initcode.into(),
            libraries: Vec::new(),
        }
    }

    /// Add a nested library dependency.
    pub fn add_library(mut self, library: DeployLibraryInput) -> Self {
        self.libraries.push(library);
        self
    }
}

impl DeployInput {
    /// Create [`DeployInput`] with the given initcode.
    ///
    /// Caller defaults to [`DEFAULT_DEPLOYER`](DEFAULT_DEPLOYER); override with [`Self::caller`].
    pub fn new(initcode: &str) -> Self {
        Self {
            caller: DEFAULT_DEPLOYER,
            value: U256::ZERO,
            initcode: initcode.into(),
            libraries: Vec::new(),
            gas_limit: u64::MAX,
        }
    }

    /// Set the account address used to deploy the contract.
    pub fn caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    /// Set the wei value sent with the deployment transaction.
    pub fn value(mut self, value: U256) -> Self {
        self.value = value;
        self
    }

    /// Add a linked library to deploy before the target contract.
    pub fn add_library(mut self, library: DeployLibraryInput) -> Self {
        self.libraries.push(library);
        self
    }

    /// Set the gas limit for the deployment transaction.
    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self
    }
}

/// Result of a deployed library.
#[derive(Debug, Clone)]
pub struct DeployLibraryOutput {
    pub id: String,
    pub address: Address,
}

/// Result of a contract deployment, including the trace.
///
/// `address` is `None` when the constructor reverts or halts, but `result`
/// and `trace` are still populated so the caller can inspect the failure.
#[derive(Debug, Clone)]
pub struct DeployOutput {
    pub address: Option<Address>,
    pub libraries: Vec<DeployLibraryOutput>,
    pub result: result::TransactionResult,
    pub trace: trace::Trace,
    pub coverage: ExecutionCoverage,
}
