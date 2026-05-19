//! Campaign orchestration: configuration, setup, and result aggregation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use tracing::{debug, error, info, trace};

pub use config::CampaignConfig;
pub use result::CampaignResult;
pub use seeds::build_seeds;

use crate::contract::{ContractArtifact, ContractBuilder};
use crate::corpus::{Corpus, CoverageMap};
use crate::fuzzer::Fuzzer;

pub mod config;
pub mod result;
pub mod seeds;

fn load_corpus_with_logging(
    dir: impl AsRef<Path>,
    seeds: &[crate::corpus::CorpusItem],
) -> std::sync::Arc<std::sync::RwLock<Corpus>> {
    let dir = dir.as_ref();
    let t0 = std::time::Instant::now();
    let c = match Corpus::load(dir) {
        Ok(c) => c,
        Err(_) => {
            let mut c = Corpus::with_seeds(seeds.to_vec());
            c.set_storage_dir(dir);
            c
        }
    };
    let elapsed = t0.elapsed();
    let sequences = c.pending.len();
    let path = dir.to_string_lossy();
    info!(target: "raptor::user", sequences = sequences, path = %path, time_ms = elapsed.as_millis(), "Loaded corpus");
    std::sync::Arc::new(std::sync::RwLock::new(c))
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

    /// Build the contract artifact and generate seeds.
    pub fn build(self) -> Result<Campaign> {
        let t0 = std::time::Instant::now();
        let artifact = ContractBuilder::build(&self.project_path, &self.contract_path)?;
        let compile_elapsed = t0.elapsed();
        info!(target: "raptor::user", name = %artifact.contract_name, time_ms = compile_elapsed.as_millis(), "Finished compiling targets");

        let seeds = build_seeds(&artifact, self.config.sequence_length);
        let fuzzed_selectors: Vec<[u8; 4]> = artifact
            .abi
            .functions()
            .filter(|f| !f.name.starts_with("invariant_"))
            .map(|f| f.selector().into())
            .collect();
        debug!(
            seed_count = seeds.len(),
            selector_count = fuzzed_selectors.len(),
            "seeds and fuzzed_selectors generated"
        );

        let corpus = self.config.corpus_dir.as_ref().map_or_else(
            || Arc::new(RwLock::new(Corpus::with_seeds(seeds.clone()))),
            |dir| load_corpus_with_logging(dir, &seeds),
        );

        Ok(Campaign {
            artifact,
            chain: None,
            corpus,
            config: self.config,
            fuzzed_selectors,
            project_path: self.project_path,
        })
    }
}

/// A fuzzing campaign that validates a target contract and orchestrates one or more fuzzers.
#[derive(Debug)]
pub struct Campaign {
    artifact: ContractArtifact,
    chain: Option<crate::chain::Chain>,
    corpus: Arc<RwLock<Corpus>>,
    config: CampaignConfig,
    fuzzed_selectors: Vec<[u8; 4]>,
    project_path: PathBuf,
}

/// Format a duration as `1m23s` or `1h2m3s`.
fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs >= 3600 {
        format!("{}h{}m{}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else if secs >= 60 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
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

    /// Deploy the target contract and run setUp() if present.
    pub fn deploy(&mut self) -> Result<()> {
        let chain = crate::chain::Chain::for_artifact(&self.artifact)
            .with_project(&self.project_path)
            .with_vm(self.config.vm.clone())
            .with_deploy_value(self.config.deploy_value)
            .with_deployer(self.config.deployer_address)
            .with_fork_config(self.config.fork_config.clone())
            .init()?
            .setup()?;
        self.chain = Some(chain);
        Ok(())
    }

    /// Run the campaign and return an aggregated result.
    pub fn run(&self) -> Result<CampaignResult> {
        let fuzzers = self.config.fuzzer_count();
        let start = std::time::Instant::now();
        let timeout = self.config.timeout_secs.map(std::time::Duration::from_secs);

        let chain = self
            .chain
            .as_ref()
            .context("campaign must be deployed before running")?;
        info!(target: "raptor::user", "Fuzzing campaign started");

        let chain = Arc::new(chain.clone());
        let artifact = self.artifact.clone();
        let config = self.config.clone();
        let fuzzed_selectors = self.fuzzed_selectors.clone();
        let corpus = self.corpus.clone();

        let fuzzers_u64 = fuzzers as u64;
        let base_runs = config.max_runs / fuzzers_u64;
        let remainder = (config.max_runs % fuzzers_u64) as usize;
        debug!(base_runs, remainder, "run distribution calculated");

        let artifact = Arc::new(artifact);
        let config = Arc::new(config);
        let fuzzed_selectors = Arc::new(fuzzed_selectors);

        // Shared atomic counters for live progress.
        let shared_runs = Arc::new(AtomicU64::new(0));
        let shared_calls = Arc::new(AtomicU64::new(0));
        let shared_gas = Arc::new(AtomicU64::new(0));
        let shared_failures = Arc::new(AtomicU64::new(0));

        // Progress reporting thread.
        let progress_shutdown = Arc::new(AtomicBool::new(false));
        let progress_handle = {
            let shutdown = Arc::clone(&progress_shutdown);
            let corpus = Arc::clone(&corpus);
            let chain = Arc::clone(&chain);
            let shared_runs = Arc::clone(&shared_runs);
            let shared_calls = Arc::clone(&shared_calls);
            let shared_gas = Arc::clone(&shared_gas);
            let shared_failures = Arc::clone(&shared_failures);
            std::thread::spawn(move || {
                let mut last_calls = 0u64;
                let mut last_gas = 0u64;
                let mut last_time = std::time::Instant::now();
                while !shutdown.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    let now = std::time::Instant::now();
                    let elapsed = now.duration_since(start);
                    let interval_secs = now.duration_since(last_time).as_secs_f64().max(1e-6);

                    let total_runs = shared_runs.load(Ordering::Relaxed);
                    let total_calls = shared_calls.load(Ordering::Relaxed);
                    let total_gas = shared_gas.load(Ordering::Relaxed);
                    let failure_count = shared_failures.load(Ordering::Relaxed);

                    let calls_delta = total_calls.saturating_sub(last_calls);
                    let gas_delta = total_gas.saturating_sub(last_gas);

                    let calls_per_sec = (calls_delta as f64 / interval_secs) as u64;
                    let gas_per_sec = (gas_delta as f64 / interval_secs) as u64;

                    let corpus_size = if let Ok(c) = corpus.read() {
                        c.items.len()
                    } else {
                        0
                    };

                    let coverage_hits = if let Ok(c) = corpus.read() {
                        c.coverage().hit_count()
                    } else {
                        0
                    };

                    let elapsed_str = format_duration(elapsed);
                    let calls_str = format!("{}({}/s)", total_calls, calls_per_sec);

                    if let Some(stats) = chain.cache_stats() {
                        info!(
                            target: "raptor::user",
                            elapsed = %elapsed_str,
                            runs = total_runs,
                            calls = %calls_str,
                            corpus = corpus_size,
                            coverage = coverage_hits,
                            failures = failure_count,
                            gas_per_sec = gas_per_sec,
                            rpc_cache_hit = stats.hits,
                            rpc_cache_miss = stats.misses,
                            "fuzz:"
                        );
                    } else {
                        info!(
                            target: "raptor::user",
                            elapsed = %elapsed_str,
                            runs = total_runs,
                            calls = %calls_str,
                            corpus = corpus_size,
                            coverage = coverage_hits,
                            failures = failure_count,
                            gas_per_sec = gas_per_sec,
                            "fuzz:"
                        );
                    }

                    last_calls = total_calls;
                    last_gas = total_gas;
                    last_time = now;
                }
            })
        };

        let mut handles = Vec::with_capacity(fuzzers);
        for fuzzer_id in 0..fuzzers {
            let local_max_runs = if fuzzer_id < remainder {
                base_runs + 1
            } else {
                base_runs
            };
            let chain = Arc::clone(&chain);
            let artifact = Arc::clone(&artifact);
            let config = Arc::clone(&config);
            let fuzzed_selectors = Arc::clone(&fuzzed_selectors);
            let corpus = Arc::clone(&corpus);
            let shared_runs = Arc::clone(&shared_runs);
            let shared_calls = Arc::clone(&shared_calls);
            let shared_gas = Arc::clone(&shared_gas);
            let shared_failures = Arc::clone(&shared_failures);

            let handle = std::thread::spawn(move || {
                let fuzzer = Fuzzer::new(artifact, chain, config, fuzzed_selectors);
                fuzzer.run(
                    corpus,
                    local_max_runs,
                    fuzzer_id,
                    start,
                    timeout,
                    shared_runs,
                    shared_calls,
                    shared_gas,
                    shared_failures,
                )
            });
            handles.push((fuzzer_id, handle));
        }

        // Join all fuzzer threads and aggregate results.
        let mut total_runs = 0u64;
        let mut total_calls = 0u64;
        let mut total_gas = 0u64;
        let mut all_failures = Vec::new();
        for (fuzzer_id, handle) in handles {
            match handle.join() {
                Ok(Ok(result)) => {
                    total_runs += result.runs;
                    total_calls += result.total_calls;
                    total_gas += result.total_gas;
                    all_failures.extend(result.failures);
                    trace!(fuzzer_id, runs = result.runs, "fuzzer joined");
                }
                Ok(Err(e)) => {
                    error!(fuzzer_id, %e, "fuzzer failed");
                }
                Err(e) => {
                    error!(fuzzer_id, ?e, "fuzzer panicked");
                }
            }
        }

        progress_shutdown.store(true, Ordering::Relaxed);
        let _ = progress_handle.join();

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

        info!(target: "raptor::user", runs = total_runs, failures = all_failures.len(), "Campaign complete");

        if let Some(ref chain) = self.chain {
            chain.flush_fork_cache();
        }

        let elapsed_secs = start.elapsed().as_secs_f64();

        Ok(CampaignResult {
            runs: total_runs,
            failures: all_failures,
            total_calls,
            total_gas,
            elapsed_secs,
            coverage,
            deployer_address: self.config.deployer_address,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::campaign::{Campaign, CampaignConfig, CampaignResult};
    use crate::contract;

    fn run_campaign(threads: usize, max_runs: u64) -> CampaignResult {
        let mut config = CampaignConfig::default();
        config.threads = threads;
        config.max_runs = max_runs;
        config.timeout_secs = Some(30);

        let mut campaign = Campaign::for_target(Path::new("test/ImpossibleBug.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap();
        campaign.deploy().unwrap();
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
        config.threads = 1;
        let mut campaign = Campaign::for_target(Path::new("test/ConstructorRevert.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap();
        let err = campaign.deploy().unwrap_err();
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
        config.threads = 1;
        let mut campaign = Campaign::for_target(Path::new("test/ComplexConstructorRevert.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap();
        let err = campaign.deploy().unwrap_err();
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
        config.threads = 1;
        let mut campaign = Campaign::for_target(Path::new("test/SetupRevert.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap();
        let err = campaign.deploy().unwrap_err();
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
            !artifact.invariants.is_empty(),
            "invariant_caught() should be discovered as an invariant"
        );

        let mut config = CampaignConfig::default();
        config.max_runs = 10_000;
        config.timeout_secs = Some(10);
        let mut campaign = Campaign::for_target(Path::new("src/L1SimpleKnob.sol"))
            .with_project(Path::new("fixtures/challenges"))
            .with_config(config)
            .build()
            .unwrap();
        campaign.deploy().unwrap();
        let result = campaign.run().unwrap();

        assert!(
            !result.failures.is_empty(),
            "raptor should find at least one crash (dragon caught)"
        );
    }

    #[test]
    fn max_runs_with_one_fuzzer() {
        let result = run_campaign(1, 1000);
        assert_eq!(result.runs, 1000, "single fuzzer should run all 1000 runs");
    }

    #[test]
    fn max_runs_with_three_fuzzers() {
        let result = run_campaign(3, 1000);
        assert_eq!(
            result.runs, 1000,
            "total runs across 3 fuzzers should be 1000"
        );
    }

    #[test]
    fn max_runs_with_four_fuzzers() {
        let result = run_campaign(4, 1000);
        assert_eq!(
            result.runs, 1000,
            "total runs across 4 fuzzers should be 1000"
        );
    }

    #[test]
    fn payable_constructor_accepts_deploy_value() {
        let mut config = CampaignConfig::default();
        config.threads = 1;
        config.max_runs = 100;
        config.timeout_secs = Some(10);
        config.deploy_value = revm::primitives::U256::from(12345);

        let mut campaign = Campaign::for_target(Path::new("test/PayableConstructor.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap();
        campaign.deploy().unwrap();
        let result = campaign.run().unwrap();

        assert!(
            result.failures.is_empty(),
            "payable constructor should accept deploy value and invariant should pass"
        );
    }

    #[test]
    fn non_payable_constructor_rejects_deploy_value() {
        let mut config = CampaignConfig::default();
        config.threads = 1;
        config.deploy_value = revm::primitives::U256::from(1);

        let mut campaign = Campaign::for_target(Path::new("test/ImpossibleBug.sol"))
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(config)
            .build()
            .unwrap();
        let err = campaign.deploy().unwrap_err();

        let msg = format!("{err}");
        assert!(
            msg.contains("revert") || msg.contains("value"),
            "non-payable constructor with non-zero value should fail deployment: {msg}"
        );
    }
}
