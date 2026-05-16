//! Campaign orchestration: configuration, setup, and result aggregation.

use std::path::{Path, PathBuf};

use anyhow::Result;
use tracing::{debug, info, instrument};

pub use config::CampaignConfig;

use crate::contract::{ContractArtifact, ContractBuilder};
use crate::worker::{PropertyFailure, Worker};

pub mod config;
pub mod input;

/// The aggregated output of a fuzzing campaign.
#[derive(Debug)]
pub struct CampaignResult {
    pub runs: u64,
    pub failures: Vec<PropertyFailure>,
}

/// Builder for constructing a [`Campaign`].
#[derive(Debug)]
pub struct CampaignBuilder {
    contract_path: PathBuf,
    project_path: PathBuf,
    config: CampaignConfig,
}

impl CampaignBuilder {
    /// Override the Foundry project root directory.
    pub fn with_project(mut self, path: impl AsRef<Path>) -> Self {
        self.project_path = path.as_ref().to_path_buf();
        self
    }

    /// Set the fuzzing configuration.
    pub fn with_config(mut self, config: CampaignConfig) -> Self {
        self.config = config;
        self
    }

    /// Build the contract artifact, validate deployment, and generate seeds.
    pub fn build(self) -> Result<Campaign> {
        let artifact = ContractBuilder::build(&self.project_path, &self.contract_path)?;
        info!(contract = %artifact.contract_name, "artifact built");

        // Validate deployment by creating a runner and immediately dropping it.
        let _runner = crate::evm::EvmRunner::from_target(&artifact)?;
        debug!("deployment validated");

        // TODO: review this call squence seed generation
        let seeds = build_seeds(&artifact, self.config.sequence_length);
        let selectors: Vec<[u8; 4]> = artifact
            .abi
            .functions()
            .map(|f| f.selector().into())
            .collect();
        debug!(
            seed_count = seeds.len(),
            selector_count = selectors.len(),
            "seeds and selectors generated"
        );

        Ok(Campaign {
            artifact,
            seeds,
            config: self.config,
            selectors,
        })
    }
}

/// A fuzzing campaign that validates a target contract and orchestrates one or more workers.
#[derive(Debug)]
pub struct Campaign {
    artifact: ContractArtifact,
    seeds: Vec<input::CallSequenceInput>,
    config: CampaignConfig,
    selectors: Vec<[u8; 4]>,
}

impl Campaign {
    /// Start building a campaign for the given target contract.
    pub fn for_target(contract: impl AsRef<Path>) -> CampaignBuilder {
        CampaignBuilder {
            contract_path: contract.as_ref().to_path_buf(),
            project_path: PathBuf::new(),
            config: CampaignConfig::default(),
        }
    }

    /// Access the loaded contract artifact.
    pub fn artifact(&self) -> &ContractArtifact {
        &self.artifact
    }

    /// Run the campaign and return an aggregated result.
    #[instrument(skip(self), fields(workers = self.config.worker_count(), max_runs = self.config.max_runs))]
    pub fn run(&self) -> Result<CampaignResult> {
        let worker = Worker::new(
            self.artifact.clone(),
            self.seeds.clone(),
            self.config.clone(),
            self.selectors.clone(),
        );
        debug!("worker created");

        info!(
            workers = self.config.worker_count(),
            "running fuzzer via Launcher"
        );
        let worker_result = worker.launch(self.config.worker_count(), self.config.broker_port)?;
        info!(
            runs = worker_result.runs,
            failures = worker_result.failures.len(),
            "worker finished"
        );

        Ok(CampaignResult {
            runs: worker_result.runs,
            failures: worker_result.failures,
        })
    }
}

/// Build seed inputs from the contract ABI.
pub fn build_seeds(artifact: &ContractArtifact, max_len: usize) -> Vec<input::CallSequenceInput> {
    let mut seeds = Vec::new();

    // Single-call seeds for every ABI function.
    for func in artifact.abi.functions() {
        let selector: [u8; 4] = func.selector().into();
        let call = input::Call {
            selector,
            args: vec![0u8; func.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        };
        seeds.push(input::CallSequenceInput::single(call));
    }

    // Combined seed with all non-view/pure action functions in ABI order.
    let action_calls: Vec<input::Call> = artifact
        .abi
        .functions()
        .filter(|f| {
            !matches!(
                f.state_mutability,
                alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
            )
        })
        .map(|f| input::Call {
            selector: f.selector().into(),
            args: vec![0u8; f.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        })
        .collect();

    if !action_calls.is_empty() {
        let mut combined = input::CallSequenceInput::new();
        combined.calls = action_calls.clone();
        seeds.push(combined);
    }

    // Permutation seeds for action functions (up to max_len).
    let n = action_calls.len();
    if n > 0 && n <= max_len {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(Vec::new());
        let mut permutations = Vec::new();
        while let Some(prefix) = queue.pop_front() {
            if prefix.len() == n {
                permutations.push(prefix);
                continue;
            }
            for (idx, _call) in action_calls.iter().enumerate() {
                let already_in_prefix = prefix.contains(&idx);
                if !already_in_prefix {
                    let mut next = prefix.to_vec();
                    next.push(idx);
                    queue.push_back(next);
                }
            }
        }
        for perm in permutations {
            let mut seq = input::CallSequenceInput::new();
            for &i in &perm {
                seq.calls.push(action_calls[i].replicate());
            }
            seeds.push(seq);
        }
    }

    seeds
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::campaign::{Campaign, CampaignConfig};
    use crate::contract;

    #[test]
    fn deployment_reports_constructor_revert_reason() {
        let _artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ConstructorRevert.sol"),
        )
        .unwrap();

        let mut config = CampaignConfig::default();
        config.workers = 1;
        let err = Campaign::for_target(Path::new("test/ConstructorRevert.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ConstructorRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_complex_constructor_trace() {
        let _artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ComplexConstructorRevert.sol"),
        )
        .unwrap();

        let mut config = CampaignConfig::default();
        config.workers = 1;
        let err = Campaign::for_target(Path::new("test/ComplexConstructorRevert.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ComplexConstructorRevertOutput.txt")
                .unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_set_up_revert_trace() {
        let _artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/SetupRevert.sol"),
        )
        .unwrap();

        let mut config = CampaignConfig::default();
        config.workers = 1;
        let err = Campaign::for_target(Path::new("test/SetupRevert.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/SetupRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn catches_l1_simple_knob_dragon() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
        .unwrap();

        assert!(
            !artifact.properties.is_empty(),
            "property_caught() should be discovered as a property"
        );

        let mut config = CampaignConfig::default();
        config.workers = 1;
        config.max_runs = 10_000;
        let campaign = Campaign::for_target(Path::new("src/L1SimpleKnob.sol"))
            .with_project(Path::new("fixtures/challenges"))
            .with_config(config)
            .build()
            .unwrap();
        let result = campaign.run().unwrap();

        assert!(
            !result.failures.is_empty(),
            "raptor should find at least one property failure (dragon caught)"
        );
    }
}
