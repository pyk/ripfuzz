//! Campaign orchestration: configuration, setup, and result aggregation.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::contract;
use crate::fuzzer;

pub mod worker;

/// Configuration for a fuzzing campaign.
#[derive(Debug, Clone)]
pub struct CampaignConfig {
    pub workers: usize,
    pub max_iters: u64,
    pub timeout_secs: u64,
    pub sequence_length: usize,
    pub seed: u64,
    pub max_block_number_delay: u64,
    pub max_block_timestamp_delay: u64,
}

impl Default for CampaignConfig {
    fn default() -> Self {
        Self {
            workers: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            max_iters: 10000,
            timeout_secs: 60,
            sequence_length: 5,
            seed: 0,
            max_block_number_delay: 5,
            max_block_timestamp_delay: 5,
        }
    }
}

impl CampaignConfig {
    /// Resolved worker count.
    pub fn worker_count(&self) -> usize {
        self.workers
    }
}

/// The aggregated output of a fuzzing campaign.
#[derive(Debug)]
pub struct CampaignResult {
    pub iterations: u64,
    pub failures: Vec<fuzzer::PropertyFailure>,
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
        let artifact = contract::ContractBuilder::build(&self.project_path, &self.contract_path)?;

        // Validate deployment by creating a runner and immediately dropping it.
        let _runner = crate::evm::EvmRunner::from_target(&artifact)?;

        let seeds = fuzzer::build_seeds(&artifact, self.config.sequence_length);
        let selectors: Vec<[u8; 4]> = artifact
            .abi
            .functions()
            .map(|f| f.selector().into())
            .collect();

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
    artifact: contract::ContractArtifact,
    seeds: Vec<fuzzer::sequence::CallSequenceInput>,
    config: CampaignConfig,
    selectors: Vec<[u8; 4]>,
}

impl Campaign {
    /// Start building a campaign for the given target contract and project.
    pub fn for_target(contract: impl AsRef<Path>, project: impl AsRef<Path>) -> CampaignBuilder {
        CampaignBuilder {
            contract_path: contract.as_ref().to_path_buf(),
            project_path: project.as_ref().to_path_buf(),
            config: CampaignConfig::default(),
        }
    }

    /// Access the loaded contract artifact.
    pub fn artifact(&self) -> &contract::ContractArtifact {
        &self.artifact
    }

    /// Run the campaign and return an aggregated result.
    pub fn run(&self) -> Result<CampaignResult> {
        let worker = worker::Worker::new(
            self.artifact.clone(),
            self.seeds.clone(),
            self.config.clone(),
            self.selectors.clone(),
        );

        let worker_result = if self.config.worker_count() == 1 {
            worker.run_single()
        } else {
            worker.run_parallel(self.config.worker_count())
        }?;

        Ok(CampaignResult {
            iterations: worker_result.iterations,
            failures: worker_result.failures,
        })
    }
}
