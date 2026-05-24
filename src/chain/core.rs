//! Core chain types: configuration, builder, and snapshot.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use alloy_json_abi::JsonAbi;
use anyhow::Result;
use revm::primitives::{Address, Bytes, U256};

use crate::chain::base_state::BaseState;
use crate::chain::error::{ChainExecutionError, ChainInitError, ChainSetupError};
use crate::chain::executor::{ExecutionOptions, execute};
use crate::chain::init::initialize;
use crate::chain::output::ExecutionOutput;
use crate::chain::setup::setup;
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
pub struct ChainBuilder<'a, V: crate::evm::cheatcode::VmFactory = crate::evm::cheatcode::Vm> {
    artifact: &'a ContractArtifact,
    project_root: PathBuf,
    vm: Option<V>,
    deploy_value: U256,
    deployer: Address,
    environment: Option<crate::chain::Environment>,
}

impl<'a, V: crate::evm::cheatcode::VmFactory> ChainBuilder<'a, V> {
    /// Override the Foundry project root directory.
    pub fn with_project(mut self, path: impl AsRef<Path>) -> Self {
        self.project_root = path.as_ref().to_path_buf();
        self
    }

    /// Set the VM component (required).
    pub fn with_vm<V2: crate::evm::cheatcode::VmFactory>(self, vm: V2) -> ChainBuilder<'a, V2> {
        ChainBuilder {
            artifact: self.artifact,
            project_root: self.project_root,
            vm: Some(vm),
            deploy_value: self.deploy_value,
            deployer: self.deployer,
            environment: self.environment,
        }
    }

    /// Set the execution environment (sandbox or fork).
    pub fn with_environment(mut self, env: crate::chain::Environment) -> Self {
        self.environment = Some(env);
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
        let vm = self.vm.ok_or_else(|| {
            ChainInitError::Other(anyhow::anyhow!("ChainBuilder::with_vm is required"))
        })?;
        let env = self
            .environment
            .unwrap_or_else(crate::chain::Environment::sandbox);
        let (contract_address, mut state) =
            initialize(self.artifact, &env, self.deploy_value, self.deployer)?;
        // Populate persistent VM config fields from the VM and artifact.
        state.project_root = vm.config().project_root.clone();
        state.ffi_enabled = vm.config().ffi;
        let initcode_map = self.artifact.initcode_map.clone();
        for (initcode, (name, _abi)) in initcode_map {
            state.compiled_contracts.insert(name, initcode);
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
    state: BaseState,
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
            vm: None,
            deploy_value: U256::ZERO,
            deployer: crate::chain::init::DEFAULT_DEPLOYER,
            environment: None,
        }
    }

    /// Create a Chain directly from a pre-built state and contract address.
    pub fn from_state(
        state: BaseState,
        contract_address: Address,
        contract_abi: JsonAbi,
        invariants: Vec<([u8; 4], String)>,
        initcode_map: HashMap<Bytes, (String, JsonAbi)>,
    ) -> Self {
        Self {
            config: ChainConfig::default(),
            state,
            contract_address,
            invariants,
            contract_abi,
            initcode_map,
        }
    }

    /// 2. Run `setup()` if present, snapshot the resulting state.
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

impl crate::chain::SequenceExecutor for Chain {
    fn execute(&self, calls: &[Call]) -> Result<ExecutionOutput> {
        self.execute_with_opts(calls, crate::chain::executor::ExecutionOptions::default())
            .map_err(Into::into)
    }
}

impl Chain {
    /// Flush the underlying database cache to disk, if one exists.
    pub fn flush_database_cache(&self) {
        if let Err(e) = self.state.flush_database_cache() {
            tracing::error!(%e, "failed to flush database cache");
        }
    }

    /// Return database cache statistics, if a fork backend is present.
    pub fn database_cache_stats(&self) -> Option<crate::chain::CacheStats> {
        self.state.db.cache_stats()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn chain_execute_returns_coverage_and_all_ok() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/basic-target", "src/NamedMismatch.sol")
                .unwrap();

        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::evm::cheatcode::Vm::new(
                crate::evm::cheatcode::VmConfig::default(),
            ))
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

    #[test]
    fn chain_from_state_skips_init() {
        let state = crate::chain::BaseState::new(crate::chain::Database::default());
        let contract_address = revm::primitives::Address::new([0xab; 20]);
        let abi = alloy_json_abi::JsonAbi::default();
        let chain = Chain::from_state(
            state.clone(),
            contract_address,
            abi.clone(),
            vec![],
            HashMap::new(),
        );
        assert_eq!(chain.contract_address(), contract_address);
        assert_eq!(chain.contract_abi(), &abi);
    }
}
