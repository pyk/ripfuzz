//! Campaign orchestration: configuration, setup, and result aggregation.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::Result;
use tracing::{debug, error, info, trace};

pub use config::CampaignConfig;
pub use result::CampaignResult;
pub use seeds::build_seeds;

use crate::contract::{ContractArtifact, ContractBuilder};
use crate::corpus::{Corpus, CoverageMap};
use crate::worker::Worker;

pub mod config;
pub mod result;
pub mod seeds;

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

        // Initialize chain once and share it across workers.
        let chain = crate::chain::Chain::initialize_with_opts(
            &artifact,
            self.project_path.clone(),
            self.config.ffi,
        )?
        .setup()?;
        debug!("deployment validated");

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

        let corpus = if let Some(ref dir) = self.config.corpus_dir {
            match Corpus::load(dir) {
                Ok(c) => Arc::new(RwLock::new(c)),
                Err(_) => {
                    let mut c = Corpus::with_seeds(seeds);
                    c.set_storage_dir(dir);
                    Arc::new(RwLock::new(c))
                }
            }
        } else {
            Arc::new(RwLock::new(Corpus::with_seeds(seeds)))
        };

        Ok(Campaign {
            artifact,
            chain,
            corpus,
            config: self.config,
            selectors,
        })
    }
}

/// A fuzzing campaign that validates a target contract and orchestrates one or more workers.
#[derive(Debug)]
pub struct Campaign {
    artifact: ContractArtifact,
    chain: crate::chain::Chain,
    corpus: Arc<RwLock<Corpus>>,
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
    pub fn run(&self) -> Result<CampaignResult> {
        let workers = self.config.worker_count();
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(self.config.timeout_secs);

        info!(workers, "starting parallel fuzzing campaign");

        let chain = Arc::new(self.chain.clone());
        let artifact = self.artifact.clone();
        let config = self.config.clone();
        let selectors = self.selectors.clone();
        let corpus = self.corpus.clone();

        let workers_u64 = workers as u64;
        let base_runs = config.max_runs / workers_u64;
        let remainder = (config.max_runs % workers_u64) as usize;
        debug!(base_runs, remainder, "run distribution calculated");

        let artifact = Arc::new(artifact);
        let config = Arc::new(config);
        let selectors = Arc::new(selectors);

        let mut handles = Vec::with_capacity(workers);
        for worker_id in 0..workers {
            let local_max_runs = if worker_id < remainder {
                base_runs + 1
            } else {
                base_runs
            };
            let chain = Arc::clone(&chain);
            let artifact = Arc::clone(&artifact);
            let config = Arc::clone(&config);
            let selectors = Arc::clone(&selectors);
            let corpus = Arc::clone(&corpus);

            let handle = std::thread::spawn(move || {
                let worker = Worker::new(artifact, chain, config, selectors);
                worker.run(corpus, local_max_runs, worker_id, start, timeout)
            });
            handles.push((worker_id, handle));
        }

        // Join all worker threads and aggregate results.
        let mut total_runs = 0u64;
        let mut all_failures = Vec::new();
        for (worker_id, handle) in handles {
            match handle.join() {
                Ok(Ok(result)) => {
                    total_runs += result.runs;
                    all_failures.extend(result.failures);
                    trace!(worker_id, runs = result.runs, "worker joined");
                }
                Ok(Err(e)) => {
                    error!(worker_id, %e, "worker failed");
                }
                Err(e) => {
                    error!(worker_id, ?e, "worker panicked");
                }
            }
        }

        // Persist corpus to disk if a directory was configured.
        if let Ok(c) = self.corpus.read()
            && let Err(e) = c.flush_to_disk()
        {
            error!(%e, "failed to flush corpus to disk");
        }

        let coverage = if let Ok(c) = self.corpus.read() {
            c.coverage().clone()
        } else {
            CoverageMap::default()
        };

        info!(
            total_runs,
            failures = all_failures.len(),
            "campaign complete"
        );

        Ok(CampaignResult {
            runs: total_runs,
            failures: all_failures,
            coverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::campaign::{Campaign, CampaignConfig, CampaignResult};
    use crate::contract;

    fn run_campaign(workers: usize, max_runs: u64) -> CampaignResult {
        let mut config = CampaignConfig::default();
        config.workers = workers;
        config.max_runs = max_runs;
        config.timeout_secs = 30;

        let campaign = Campaign::for_target(Path::new("test/ImpossibleBug.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap();
        campaign.run().unwrap()
    }

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
        config.max_runs = 10_000;
        config.timeout_secs = 10;
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

    #[test]
    fn max_runs_with_one_worker() {
        let result = run_campaign(1, 1000);
        assert_eq!(result.runs, 1000, "single worker should run all 1000 runs");
    }

    #[test]
    fn max_runs_with_three_workers() {
        let result = run_campaign(3, 1000);
        assert_eq!(
            result.runs, 1000,
            "total runs across 3 workers should be 1000"
        );
    }

    #[test]
    fn max_runs_with_four_workers() {
        let result = run_campaign(4, 1000);
        assert_eq!(
            result.runs, 1000,
            "total runs across 4 workers should be 1000"
        );
    }
}
