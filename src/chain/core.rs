//! Core chain types: configuration, builder, and snapshot.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
pub struct ChainBuilder<'a, V: crate::vm::VmFactory = crate::vm::Vm> {
    artifact: &'a ContractArtifact,
    project_root: PathBuf,
    vm: Option<V>,
    vm_state: Option<crate::vm::VmState>,
    deploy_value: U256,
    deployer: Address,
    rpc: Option<Arc<dyn crate::rpc::RpcClient>>,
    fork_config: Option<crate::chain::fork::ForkConfig>,
}

impl<'a, V: crate::vm::VmFactory> ChainBuilder<'a, V> {
    /// Override the Foundry project root directory.
    pub fn with_project(mut self, path: impl AsRef<Path>) -> Self {
        self.project_root = path.as_ref().to_path_buf();
        self
    }

    /// Set the VM component (required).
    pub fn with_vm<V2: crate::vm::VmFactory>(self, vm: V2) -> ChainBuilder<'a, V2> {
        ChainBuilder {
            artifact: self.artifact,
            project_root: self.project_root,
            vm: Some(vm),
            vm_state: self.vm_state,
            deploy_value: self.deploy_value,
            deployer: self.deployer,
            rpc: self.rpc,
            fork_config: self.fork_config,
        }
    }

    /// Inject a pre-built VmState (overrides the VM's fresh_state).
    pub fn with_vm_state(mut self, state: crate::vm::VmState) -> Self {
        self.vm_state = Some(state);
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

    /// Set the optional fork configuration.
    pub fn with_fork_config(mut self, config: Option<crate::chain::fork::ForkConfig>) -> Self {
        self.fork_config = config;
        self
    }

    /// Set the pre-built RPC client (required when `fork_config` is set).
    pub fn with_rpc(mut self, rpc: Option<Arc<dyn crate::rpc::RpcClient>>) -> Self {
        self.rpc = rpc;
        self
    }

    /// Deploy the contract, verify deployment success, and return a [`Chain`].
    pub fn init(self) -> Result<Chain, ChainInitError> {
        let vm = self.vm.ok_or_else(|| {
            ChainInitError::Other(anyhow::anyhow!("ChainBuilder::with_vm is required"))
        })?;
        let (contract_address, mut state) = initialize(
            self.artifact,
            self.project_root.clone(),
            vm.config().ffi,
            self.deploy_value,
            self.deployer,
            self.rpc.as_ref(),
            self.fork_config.as_ref(),
        )?;
        // Use injected VmState if provided, otherwise use fresh state from Vm.
        if let Some(vm_state) = self.vm_state {
            state.vm = vm_state;
        } else {
            state.vm = vm.fresh_state();
        }
        // Populate compiled-contract map for vm.getCode lookups.
        let initcode_map = self.artifact.initcode_map.clone();
        for (initcode, (name, _abi)) in initcode_map {
            state.vm.compiled_contracts.insert(name, initcode);
        }
        state.vm.project_root = self.project_root;
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
            vm: None,
            vm_state: None,
            deploy_value: U256::ZERO,
            deployer: crate::chain::init::DEFAULT_DEPLOYER,
            rpc: None,
            fork_config: None,
        }
    }

    /// Create a Chain directly from a pre-built state and contract address.
    pub fn from_state(
        state: ChainState,
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

impl crate::chain::SequenceExecutor for Chain {
    fn execute(&self, calls: &[Call]) -> anyhow::Result<ExecutionOutput> {
        self.execute_with_opts(calls, crate::chain::executor::ExecutionOptions::default())
            .map_err(Into::into)
    }
}

impl Chain {
    /// Flush the underlying fork cache to disk, if one exists.
    pub fn flush_fork_cache(&self) {
        if let Err(e) = self.state.flush_fork_cache() {
            tracing::error!(%e, "failed to flush fork cache");
        }
    }

    /// Return fork cache statistics, if a fork backend is present.
    pub fn cache_stats(&self) -> Option<crate::chain::fork::CacheStats> {
        self.state.db.db.cache_stats()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
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
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
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
        let state = crate::chain::state::ChainState::new(crate::chain::fork::ForkDatabase::new(
            crate::chain::fork::ForkBackend::empty(),
        ));
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
