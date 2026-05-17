//! Chain abstraction: composable, functional EVM integration.
//!
//! Provides a single entry point for deployment, setup, and sequence execution.

use std::collections::HashMap;

use alloy_json_abi::JsonAbi;
use revm::primitives::{Address, Bytes};

use error::{ChainExecutionError, ChainInitError, ChainSetupError};
use executor::{ExecutionOptions, execute};
use init::initialize;
use output::ExecutionOutput;
use setup::setup;
use state::ChainState;

use crate::contract::ContractArtifact;
use crate::corpus::Call;

pub mod cheatcodes;
pub mod error;
pub mod executor;
pub mod init;
pub mod inspectors;
pub mod output;
pub mod setup;
pub mod state;

/// Configuration for chain execution.
#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub caller: Address,
    pub gas_limit: u64,
    pub max_sequence_calls: usize,
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            caller: init::CALLER,
            gas_limit: init::GAS_LIMIT,
            max_sequence_calls: 5,
        }
    }
}

/// Immutable chain snapshot after deployment and optional setup.
#[derive(Debug, Clone)]
pub struct Chain {
    config: ChainConfig,
    state: ChainState,
    contract_address: Address,
    properties: Vec<([u8; 4], String)>,
    contract_abi: JsonAbi,
    initcode_map: HashMap<Bytes, (String, JsonAbi)>,
}

impl Chain {
    /// 1. Compile initcode, deploy, verify deployment success.
    pub fn initialize(artifact: &ContractArtifact) -> Result<Self, ChainInitError> {
        let (contract_address, mut state) = initialize(artifact)?;
        // Populate compiled-contract map for vm.getCode lookups.
        state
            .cheatcodes
            .compiled_contracts
            .insert(artifact.contract_name.clone(), artifact.initcode.clone());
        Ok(Self {
            config: ChainConfig::default(),
            state,
            contract_address,
            properties: artifact.properties.clone(),
            contract_abi: artifact.abi.clone(),
            initcode_map: artifact.initcode_map.clone(),
        })
    }

    /// 2. Run `setUp()` if present, snapshot the resulting state.
    pub fn setup(mut self) -> Result<Self, ChainSetupError> {
        let new_state = setup(
            self.state,
            self.contract_address,
            &self.contract_abi,
            &self.initcode_map,
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
            &self.properties,
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

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
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
    fn cheatcode_warp_label_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/CheatcodeWarpLabelPrank.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        // `action()` selector = keccak256("action()")[:4]
        let action_selector: [u8; 4] = [0x0a, 0x7a, 0x1c, 0x4d];
        let calls = vec![Call {
            selector: action_selector,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain
            .execute_with_opts(
                &calls,
                crate::chain::executor::ExecutionOptions { trace: true },
            )
            .unwrap();
        let trace = output.trace.expect("trace enabled");
        let formatted = trace.format();
        assert!(
            output.all_ok,
            "action() should succeed. trace:\n{formatted}\nproperty_results: {:?}",
            output.property_results
        );
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "all properties should pass. trace:\n{formatted}"
        );

        // Trace should contain the label set during setUp.
        assert!(
            formatted.contains("TargetContract"),
            "trace should show the vm.label name:\n{formatted}"
        );
    }

    #[test]
    fn cheatcode_snapshot_revert_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/CheatcodeSnapshotRevert.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        // `increment()` selector = keccak256("increment()")[:4]
        let inc_selector: [u8; 4] = [0xd0, 0x9d, 0xe0, 0x8a];

        let calls = vec![
            Call {
                selector: inc_selector,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: inc_selector,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            Call {
                selector: inc_selector,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "all increment() calls should succeed");
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "counter should be 3, never 100"
        );
    }
}
