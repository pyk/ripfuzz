//! Core chain types: configuration, builder, and snapshot.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use alloy_json_abi::JsonAbi;
use revm::primitives::{Address, Bytes, U256};

use crate::chain::error::{ChainExecutionError, ChainInitError, ChainSetupError};
use crate::chain::executor::{ExecutionOptions, execute};
use crate::chain::init::initialize;
use crate::chain::output::ExecutionOutput;
use crate::chain::setup::setup;
use crate::chain::state::ChainState;
use crate::contract::ContractArtifact;
use crate::corpus::Call;

/// Configuration for chain execution.
#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub caller: Address,
    pub max_sequence_calls: usize,
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            caller: crate::chain::init::DEFAULT_DEPLOYER,
            max_sequence_calls: 32,
        }
    }
}

/// Builder for constructing a [`Chain`].
#[derive(Debug)]
pub struct ChainBuilder<'a> {
    artifact: &'a ContractArtifact,
    project_root: PathBuf,
    ffi_enabled: bool,
    deploy_value: U256,
    deployer: Address,
}

impl<'a> ChainBuilder<'a> {
    /// Override the Foundry project root directory.
    pub fn with_project(mut self, path: impl AsRef<Path>) -> Self {
        self.project_root = path.as_ref().to_path_buf();
        self
    }

    /// Enable the `ffi` cheatcode.
    pub fn with_ffi(mut self, enabled: bool) -> Self {
        self.ffi_enabled = enabled;
        self
    }

    /// Set the wei value sent with the deployment transaction.
    pub fn with_deploy_value(mut self, value: U256) -> Self {
        self.deploy_value = value;
        self
    }

    /// Set the account address used to deploy the target contract.
    pub fn with_deployer(mut self, deployer: Address) -> Self {
        self.deployer = deployer;
        self
    }

    /// Deploy the contract, verify deployment success, and return a [`Chain`].
    pub fn init(self) -> Result<Chain, ChainInitError> {
        let (contract_address, mut state) = initialize(
            self.artifact,
            self.project_root,
            self.ffi_enabled,
            self.deploy_value,
            self.deployer,
        )?;
        // Populate compiled-contract map for vm.getCode lookups.
        let initcode_map = self.artifact.initcode_map.clone();
        for (initcode, (name, _abi)) in initcode_map {
            state.cheatcodes.compiled_contracts.insert(name, initcode);
        }
        Ok(Chain {
            config: ChainConfig {
                caller: self.deployer,
                ..ChainConfig::default()
            },
            state,
            contract_address,
            invariants: self.artifact.invariants.clone(),
            contract_abi: self.artifact.abi.clone(),
            initcode_map: self.artifact.initcode_map.clone(),
        })
    }
}

/// Immutable chain snapshot after deployment and optional setup.
#[derive(Debug, Clone)]
pub struct Chain {
    config: ChainConfig,
    state: ChainState,
    contract_address: Address,
    invariants: Vec<([u8; 4], String)>,
    contract_abi: JsonAbi,
    initcode_map: HashMap<Bytes, (String, JsonAbi)>,
}

impl Chain {
    /// Start building a chain for the given contract artifact.
    pub fn for_artifact(artifact: &ContractArtifact) -> ChainBuilder<'_> {
        ChainBuilder {
            artifact,
            project_root: PathBuf::new(),
            ffi_enabled: false,
            deploy_value: U256::ZERO,
            deployer: crate::chain::init::DEFAULT_DEPLOYER,
        }
    }

    /// 2. Run `setUp()` if present, snapshot the resulting state.
    pub fn setup(mut self) -> Result<Self, ChainSetupError> {
        let new_state = setup(
            self.state,
            self.contract_address,
            &self.contract_abi,
            &self.initcode_map,
            self.config.caller,
        )?;
        self.state = new_state;
        Ok(self)
    }

    /// 3. Execute a call sequence against a cloned post-setup state.
    pub fn execute(&self, calls: &[Call]) -> Result<ExecutionOutput, ChainExecutionError> {
        self.execute_with_opts(calls, ExecutionOptions::default())
    }

    /// Execute with explicit options.
    pub fn execute_with_opts(
        &self,
        calls: &[Call],
        opts: ExecutionOptions,
    ) -> Result<ExecutionOutput, ChainExecutionError> {
        execute(
            &self.state,
            self.contract_address,
            &self.invariants,
            &self.contract_abi,
            &self.config,
            &self.initcode_map,
            calls,
            opts,
        )
    }

    /// Access the deployed contract address.
    pub fn contract_address(&self) -> Address {
        self.contract_address
    }

    /// Access the contract ABI.
    pub fn contract_abi(&self) -> &JsonAbi {
        &self.contract_abi
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn chain_execute_returns_coverage_and_all_ok() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("src/NamedMismatch.sol"),
        )
        .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .init()
            .unwrap()
            .setup()
            .unwrap();
        // `set(uint256)` selector = keccak256("set(uint256)")[:4]
        let set_selector: [u8; 4] = [0x60, 0xfe, 0x47, 0xb1];
        let calls = vec![Call {
            selector: set_selector,
            args: vec![0u8; 32], // x = 0
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(
            !output.coverage.contracts.is_empty(),
            "coverage should contain at least one contract"
        );
        assert!(output.all_ok, "set(0) call should succeed");
    }
}
