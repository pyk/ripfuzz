//! Core campaign types: builder, execution, and result aggregation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use tracing::{debug, error, info, trace};

use crate::campaign::config::CampaignConfig;
use crate::campaign::result::CampaignResult;
use crate::campaign::seeds::build_seeds;
use crate::chain::Chain;
use crate::contract::ContractArtifact;
use crate::corpus::Corpus;
use crate::evm::coverage::map::CoverageMap;
/// Builder for constructing a [`Campaign`].
#[derive(Debug)]
pub struct CampaignBuilder {
    artifact: Option<ContractArtifact>,
    chain: Option<Chain>,
    corpus: Option<Arc<RwLock<Corpus>>>,
    project_path: PathBuf,
    config: CampaignConfig,
    fuzzer_factory: Option<Arc<dyn crate::fuzzer::FuzzerFactory>>,
}

impl Default for CampaignBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CampaignBuilder {
    pub fn new() -> Self {
        Self {
            artifact: None,
            chain: None,
            corpus: None,
            project_path: PathBuf::new(),
            config: CampaignConfig::default(),
            fuzzer_factory: None,
        }
    }

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

    /// Inject a pre-built contract artifact.
    pub fn with_artifact(mut self, artifact: ContractArtifact) -> Self {
        self.artifact = Some(artifact);
        self
    }

    /// Inject a pre-built chain (already deployed and set up).
    pub fn with_chain(mut self, chain: Chain) -> Self {
        self.chain = Some(chain);
        self
    }

    /// Inject a pre-built corpus.
    pub fn with_corpus(mut self, corpus: Arc<RwLock<Corpus>>) -> Self {
        self.corpus = Some(corpus);
        self
    }

    /// Inject a fuzzer factory (required).
    pub fn with_fuzzer(mut self, fuzzer: impl crate::fuzzer::FuzzerFactory + 'static) -> Self {
        self.fuzzer_factory = Some(Arc::new(fuzzer));
        self
    }

    /// Build the campaign from the injected dependencies.
    pub fn build(self) -> Result<Campaign> {
        let artifact = self
            .artifact
            .context("CampaignBuilder::with_artifact is required")?;

        let fuzzed_selectors: Vec<[u8; 4]> = artifact
            .abi
            .functions()
            .filter(|f| !f.name.starts_with("invariant_"))
            .map(|f| f.selector().into())
            .collect();
        debug!(
            selector_count = fuzzed_selectors.len(),
            "fuzzed_selectors generated"
        );

        let corpus = if let Some(corpus) = self.corpus {
            corpus
        } else {
            let seeds = build_seeds(&artifact, self.config.sequence_length);
            Arc::new(RwLock::new(Corpus::with_seeds(seeds)))
        };

        let chain = self
            .chain
            .context("CampaignBuilder::with_chain is required")?;
        let fuzzer_factory = self
            .fuzzer_factory
            .context("CampaignBuilder::with_fuzzer is required")?;

        Ok(Campaign {
            artifact,
            chain,
            corpus,
            config: self.config,
            fuzzed_selectors,
            project_path: self.project_path,
            fuzzer_factory,
        })
    }
}

/// A fuzzing campaign that validates a target contract and orchestrates one or more fuzzers.
#[derive(Debug)]
pub struct Campaign {
    artifact: ContractArtifact,
    chain: Chain,
    corpus: Arc<RwLock<Corpus>>,
    config: CampaignConfig,
    fuzzed_selectors: Vec<[u8; 4]>,
    project_path: PathBuf,
    fuzzer_factory: Arc<dyn crate::fuzzer::FuzzerFactory>,
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
    /// Access the loaded contract artifact.
    pub fn artifact(&self) -> &ContractArtifact {
        &self.artifact
    }

    /// Access the project path.
    pub fn project_path(&self) -> &Path {
        &self.project_path
    }

    /// Run the campaign and return an aggregated result.
    pub fn run(&self) -> Result<CampaignResult> {
        let fuzzers = self.config.fuzzer_count();
        let start = std::time::Instant::now();
        let timeout = self.config.timeout_secs.map(std::time::Duration::from_secs);

        info!("Fuzzing campaign started");

        let chain = Arc::new(self.chain.clone());
        let artifact = self.artifact.clone();
        let config = self.config.clone();
        let fuzzed_selectors = self.fuzzed_selectors.clone();
        let corpus = self.corpus.clone();

        let fuzzers_u64 = fuzzers as u64;
        let base_runs = config.max_runs / fuzzers_u64;
        let remainder = (config.max_runs % fuzzers_u64) as usize;
        debug!(base_runs, remainder, "run distribution calculated");

        let artifact = Arc::new(artifact);
        let fuzzer_config = crate::fuzzer::config::FuzzerConfig {
            seed: config.seed,
            sequence_length: config.sequence_length,
            max_block_number_delay: config.max_block_number_delay,
            max_block_timestamp_delay: config.max_block_timestamp_delay,
        };
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

                    if let Some(stats) = chain.database_cache_stats() {
                        info!(
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
            let fuzzed_selectors = Arc::clone(&fuzzed_selectors);
            let corpus = Arc::clone(&corpus);
            let shared_runs = Arc::clone(&shared_runs);
            let shared_calls = Arc::clone(&shared_calls);
            let shared_gas = Arc::clone(&shared_gas);
            let shared_failures = Arc::clone(&shared_failures);
            let factory = Arc::clone(&self.fuzzer_factory);

            let handle = std::thread::spawn(move || {
                let fuzzer = factory.create(artifact, chain, fuzzer_config, fuzzed_selectors);
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

        info!(
            runs = total_runs,
            failures = all_failures.len(),
            "Campaign complete"
        );

        self.chain.flush_database_cache();

        let elapsed_secs = start.elapsed().as_secs_f64();

        Ok(CampaignResult {
            runs: total_runs,
            failures: all_failures,
            total_calls,
            total_gas,
            elapsed_secs,
            coverage,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use crate::campaign::{CampaignBuilder, CampaignConfig, CampaignResult};
    use crate::contract;

    fn run_campaign(threads: usize, max_runs: u64) -> CampaignResult {
        let mut config = CampaignConfig::default();
        config.threads = threads;
        config.max_runs = max_runs;
        config.timeout_secs = Some(30);

        let artifact =
            contract::tests::load_test_artifact("fixtures/basic-target", "test/ImpossibleBug.sol")
                .unwrap();
        let chain = crate::chain::Chain::for_artifact(&artifact)
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(crate::evm::cheatcode::Config::default())
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let campaign = CampaignBuilder::new()
            .with_project(Path::new("fixtures/basic-target"))
            .with_artifact(artifact)
            .with_chain(chain)
            .with_config(config)
            .with_fuzzer(crate::fuzzer::DefaultFuzzerFactory)
            .build()
            .unwrap();
        campaign.run().unwrap()
    }

    #[test]
    fn deployment_reports_constructor_revert_reason() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/basic-target",
            "test/ConstructorRevert.sol",
        )
        .unwrap();

        let err = crate::chain::Chain::for_artifact(&artifact)
            .with_config(crate::evm::cheatcode::Config::default())
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(crate::evm::cheatcode::Config::default())
            .init()
            .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ConstructorRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_complex_constructor_trace() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/basic-target",
            "test/ComplexConstructorRevert.sol",
        )
        .unwrap();

        let err = crate::chain::Chain::for_artifact(&artifact)
            .with_config(crate::evm::cheatcode::Config::default())
            .with_project(Path::new("fixtures/basic-target"))
            .init()
            .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ComplexConstructorRevertOutput.txt")
                .unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_set_up_revert_trace() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/basic-target", "test/SetupRevert.sol")
                .unwrap();

        let err = crate::chain::Chain::for_artifact(&artifact)
            .with_config(crate::evm::cheatcode::Config::default())
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(crate::evm::cheatcode::Config::default())
            .init()
            .unwrap()
            .setup()
            .unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/SetupRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn catches_l1_simple_knob_dragon() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/challenges", "src/L1SimpleKnob.sol")
                .unwrap();

        assert!(
            !artifact.invariants.is_empty(),
            "invariant_caught() should be discovered as an invariant"
        );

        let mut config = CampaignConfig::default();
        config.max_runs = 10_000;
        config.timeout_secs = Some(10);
        let chain = crate::chain::Chain::for_artifact(&artifact)
            .with_project(Path::new("fixtures/challenges"))
            .with_config(crate::evm::cheatcode::Config::default())
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let campaign = CampaignBuilder::new()
            .with_project(Path::new("fixtures/challenges"))
            .with_artifact(artifact)
            .with_chain(chain)
            .with_config(config)
            .with_fuzzer(crate::fuzzer::DefaultFuzzerFactory)
            .build()
            .unwrap();
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
        let deploy_value = revm::primitives::U256::from(12345);

        let artifact = contract::tests::load_test_artifact(
            "fixtures/basic-target",
            "test/PayableConstructor.sol",
        )
        .unwrap();
        let chain = crate::chain::Chain::for_artifact(&artifact)
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(crate::evm::cheatcode::Config::default())
            .with_deploy_value(deploy_value)
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let campaign = CampaignBuilder::new()
            .with_project(Path::new("fixtures/basic-target"))
            .with_artifact(artifact)
            .with_chain(chain)
            .with_config(config)
            .with_fuzzer(crate::fuzzer::DefaultFuzzerFactory)
            .build()
            .unwrap();
        let result = campaign.run().unwrap();

        assert!(
            result.failures.is_empty(),
            "payable constructor should accept deploy value and invariant should pass"
        );
    }

    #[test]
    fn non_payable_constructor_rejects_deploy_value() {
        let deploy_value = revm::primitives::U256::from(1);

        let artifact =
            contract::tests::load_test_artifact("fixtures/basic-target", "test/ImpossibleBug.sol")
                .unwrap();
        let err = crate::chain::Chain::for_artifact(&artifact)
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(crate::evm::cheatcode::Config::default())
            .with_deploy_value(deploy_value)
            .init()
            .unwrap_err();

        let msg = format!("{err}");
        assert!(
            msg.contains("revert") || msg.contains("value"),
            "non-payable constructor with non-zero value should fail deployment: {msg}"
        );
    }

    #[test]
    fn campaign_without_contract_builder() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/basic-target", "test/ImpossibleBug.sol")
                .unwrap();
        let chain = crate::chain::Chain::for_artifact(&artifact)
            .with_project(Path::new("fixtures/basic-target"))
            .with_config(crate::evm::cheatcode::Config::default())
            .init()
            .unwrap()
            .setup()
            .unwrap();

        let mut config = CampaignConfig::default();
        config.threads = 1;
        config.max_runs = 10;
        config.timeout_secs = Some(10);

        let campaign = CampaignBuilder::new()
            .with_project(Path::new("fixtures/basic-target"))
            .with_artifact(artifact)
            .with_chain(chain)
            .with_config(config)
            .with_fuzzer(crate::fuzzer::DefaultFuzzerFactory)
            .build()
            .unwrap();

        let result = campaign.run().unwrap();
        assert_eq!(result.runs, 10);
    }
}
