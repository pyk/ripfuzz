//! `fuzz` CLI command implementation.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use alloy_primitives::Address;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use revm::primitives::{Bytes, U256};
use tracing::{debug, info, instrument};

use crate::evm::coverage::SharedCoverage;
use crate::fuzzer::corpus::*;
use crate::fuzzer::{CorpusReplayer, SharedMetrics};
use crate::*;

fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn parse_threads(s: &str) -> Result<usize, String> {
    let n = s
        .parse::<usize>()
        .map_err(|e| format!("invalid thread count: {e}"))?;
    if n == 0 {
        return Err("threads must be at least 1".into());
    }
    Ok(n)
}

fn parse_balance(s: &str) -> Result<U256, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(U256::ZERO);
    }

    let lower = trimmed.to_lowercase();
    if let Some(stripped) = lower.strip_prefix("0x") {
        return U256::from_str_radix(stripped, 16).map_err(|e| format!("invalid hex balance: {e}"));
    }

    if trimmed.contains(['e', 'E']) {
        let f = trimmed
            .parse::<f64>()
            .map_err(|e| format!("invalid scientific notation balance: {e}"))?;
        let plain = format!("{:.0}", f);
        return U256::from_str_radix(&plain, 10)
            .map_err(|e| format!("invalid scientific notation balance: {e}"));
    }

    U256::from_str_radix(trimmed, 10).map_err(|e| format!("invalid decimal balance: {e}"))
}

fn parse_address(s: &str) -> Result<Address, String> {
    let trimmed = s.trim();
    let mut hex = String::from(trimmed.trim_start_matches("0x").trim_start_matches("0X"));
    if !hex.len().is_multiple_of(2) {
        hex.insert(0, '0');
    }
    let bytes = hex::decode(&hex).map_err(|e| format!("invalid hex address: {e}"))?;
    if bytes.len() > 20 {
        return Err("address exceeds 20 bytes".into());
    }
    let mut padded = [0u8; 20];
    padded[20 - bytes.len()..].copy_from_slice(&bytes);
    Ok(Address::new(padded))
}

#[derive(Debug, Parser)]
pub struct Args {
    /// Target contract identifier (e.g. ./test/Contract.sol:Contract).
    #[arg(value_name = "TARGET")]
    pub target: foundry::ArtifactId,

    // Project & Deployment
    /// Path to the Foundry project root.
    #[arg(
        short = 'p',
        long = "project",
        value_name = "PATH",
        help_heading = "Project & Deployment"
    )]
    pub project_path: Option<PathBuf>,

    /// Wei to send during target contract deployment.
    #[arg(long = "deploy-value", default_value = "0", value_parser = parse_balance, value_name = "WEI", help_heading = "Project & Deployment")]
    pub deploy_value: U256,

    /// Account address used to deploy the target contract.
    #[arg(
        long = "deployer",
        default_value_t = crate::evm::chain::DEFAULT_DEPLOYER,
        value_parser = parse_address,
        value_name = "ADDRESS",
        help_heading = "Project & Deployment"
    )]
    pub deployer_address: Address,

    // Campaign Limits
    /// Number of parallel fuzzer threads to spawn.
    #[arg(short = 'w', long = "threads", default_value_t = default_threads(), value_parser = parse_threads, value_name = "N", help_heading = "Campaign Limits")]
    pub threads: usize,

    /// Maximum number of campaign runs across all fuzzers.
    #[arg(
        short = 'r',
        long = "max-runs",
        default_value = "10000",
        value_name = "N",
        help_heading = "Campaign Limits"
    )]
    pub max_runs: u64,

    /// Timeout in seconds for the entire fuzzing campaign.
    #[arg(
        short = 't',
        long = "timeout",
        value_name = "SECS",
        help_heading = "Campaign Limits"
    )]
    pub timeout_secs: Option<u64>,

    /// Maximum number of calls in each generated fuzzing sequence.
    #[arg(
        short = 'c',
        long = "max-calls",
        default_value = "32",
        value_name = "N",
        help_heading = "Fuzzing Parameters"
    )]
    pub max_calls: usize,

    /// Random seed for reproducibility.
    #[arg(
        long = "seed",
        default_value = "0",
        value_name = "N",
        help_heading = "Fuzzing Parameters"
    )]
    pub seed: u64,

    // Corpus
    /// Directory to load and persist coverage-guided corpus files.
    #[arg(long = "corpus-dir", value_name = "DIR", help_heading = "Corpus")]
    pub corpus_dir: Option<PathBuf>,

    // Logging
    #[command(flatten)]
    pub verbosity: clap_verbosity_flag::Verbosity<clap_verbosity_flag::InfoLevel>,

    /// Fork mode configuration.
    #[command(flatten)]
    pub fork_mode: ForkModeArgs,

    // Security
    /// Enable the `ffi` cheatcode (security-sensitive).
    #[arg(long = "ffi", help_heading = "Security")]
    pub ffi: bool,

    // Foundry
    /// Skip cache and force recompilation.
    #[arg(long = "force", help_heading = "Foundry")]
    pub force: bool,
}

#[derive(Debug, Parser)]
pub struct ForkModeArgs {
    /// JSON-RPC URL to fork from.
    #[arg(long = "rpc-url", value_name = "URL", help_heading = "Fork Mode")]
    pub rpc_url: Option<String>,

    /// Block number to fork at. Must be <= the remote latest block.
    #[arg(long = "rpc-block", value_name = "N", help_heading = "Fork Mode")]
    pub rpc_block: Option<u64>,

    /// Maximum retry attempts per RPC URL after transient failure.
    #[arg(
        long = "rpc-retries",
        default_value = "3",
        value_name = "N",
        help_heading = "Fork Mode"
    )]
    pub rpc_retries: u32,

    /// Initial retry backoff in milliseconds (doubles each attempt).
    #[arg(
        long = "rpc-backoff",
        default_value = "100",
        value_name = "MS",
        help_heading = "Fork Mode"
    )]
    pub rpc_backoff: u64,

    /// Optional rate limit: maximum requests per second across all URLs.
    #[arg(long = "rpc-rate-limit", value_name = "N", help_heading = "Fork Mode")]
    pub rpc_rate_limit: Option<u64>,

    /// Request timeout in milliseconds for each RPC call.
    #[arg(
        long = "rpc-timeout",
        default_value = "30000",
        value_name = "MS",
        help_heading = "Fork Mode"
    )]
    pub rpc_timeout: u64,
}

impl Default for ForkModeArgs {
    fn default() -> Self {
        Self {
            rpc_url: None,
            rpc_block: None,
            rpc_retries: 3,
            rpc_backoff: 100,
            rpc_rate_limit: None,
            rpc_timeout: 30000,
        }
    }
}

impl ForkModeArgs {
    /// Build a [`forkdb::Config`](crate::evm::forkdb::Config) from CLI arguments.
    pub fn build_fork_config(&self, project_path: impl AsRef<Path>) -> Result<evm::forkdb::Config> {
        let cache_dir = project_path.as_ref().join("raptor").join("cache");
        let block = self
            .rpc_block
            .context("--rpc-block is required with --rpc-url")?;
        let url = self.rpc_url.as_ref().context("--rpc-url is required")?;
        let config = evm::forkdb::Config::new(url.clone())
            .retries(self.rpc_retries)
            .backoff_ms(self.rpc_backoff)
            .rate_limit(self.rpc_rate_limit)
            .timeout_ms(self.rpc_timeout)
            .cache_dir(&cache_dir)
            .block_number(block);
        Ok(config)
    }
}

#[instrument(skip(args), fields(target = ?args.target, threads = args.threads, max_runs = args.max_runs))]
pub fn run(args: Args) -> Result<()> {
    // Resolve project path
    let project_path = args.project_path.map(Ok).unwrap_or_else(env::current_dir)?;
    debug!(?project_path, "resolved project path");

    // Build project
    info!("building project");
    let project = foundry::Project::new(&project_path);
    let build_opts = foundry::BuildOptions::new().force(args.force);
    project.build(build_opts)?;

    // Load build artifacts
    info!("loading build artifacts");
    let build_artifacts = project.load_artifacts()?;
    ensure!(
        build_artifacts.contains_key(&args.target),
        "target artifact `{}` not found in build artifacts",
        args.target
    );

    // Load target contract and prepare library dependencies.
    info!("loading target contract");
    let target_contract = evm::Contract::try_get(&build_artifacts, &args.target)?;
    if !target_contract.libraries.is_empty() {
        let lib_ids: Vec<&str> = target_contract
            .libraries
            .iter()
            .map(|l| l.id.as_str())
            .collect();
        info!("linked external libraries: {:?}", lib_ids);
    }

    // TODO(pyk): Create InitcodeRegistry
    // Build compiled-contract registry for vm.getCode
    let mut compiled_contracts = HashMap::new();
    for (id, artifact) in &build_artifacts {
        let bytecode = match artifact {
            foundry::Artifact::Contract(c) => &c.bytecode.object,
            foundry::Artifact::Library(c) => &c.bytecode.object,
            _ => continue,
        };
        let initcode: Bytes = bytecode.parse().unwrap_or_default();
        if initcode.is_empty() {
            continue;
        }
        compiled_contracts.insert(id.into(), initcode);
    }

    // Create test chain
    info!("creating test chain");
    let mut chain_config = evm::chain::Config::new(&project_path)
        .with_compiled_contracts(compiled_contracts)
        .coverage(true);
    if args.fork_mode.rpc_url.is_some() {
        let fork_config = args.fork_mode.build_fork_config(&project_path)?;
        chain_config = chain_config.fork(fork_config);
        info!("forking a chain"); // TODO: add chain name, block number etc
    }
    let mut chain = evm::Chain::new(chain_config)?;

    // Deploy target contract
    info!("deploying target contract");
    let mut deploy_opts = evm::DeployInput::new(&target_contract.initcode)
        .caller(args.deployer_address)
        .value(args.deploy_value);
    let libraries = target_contract.libraries.clone();
    for lib in libraries {
        deploy_opts = deploy_opts.add_library(lib);
    }
    let deployment = chain.deploy(deploy_opts)?;
    ensure!(
        deployment.result.success,
        "target contract deployment failed (output: {:?})\n\ntrace:\n{:#?}",
        deployment.result.output,
        deployment.trace
    );
    let deployed_address = deployment
        .address
        .context("deployment succeeded but created_address is missing")?;
    info!(%deployed_address, "target contract deployed");

    // Run setup if present
    if let Some(ref setup) = target_contract.setup_function {
        info!("calling setup");
        let setup_opts = evm::SetupInput::new(deployed_address)
            .calldata(Bytes::from(setup.selector().as_slice().to_vec()))
            .caller(args.deployer_address);
        let setup_output = chain.setup(setup_opts)?;
        ensure!(
            setup_output.result.success,
            "setup failed (output: {:?})\n\ntrace:\n{:#?}",
            setup_output.result.output,
            setup_output.trace
        );
        info!("setup completed");
    }

    // Initialize shared corpus
    // Extract literals from build artifacts so the fuzzer can seed random value
    // generation with concrete values found across the entire project.
    let literals = ExtractedLiterals::from_artifacts(&build_artifacts);
    let base_corpus_dir = args
        .corpus_dir
        .unwrap_or_else(|| project_path.join("raptor").join("corpus"));
    let corpus_dir = SharedCorpus::dir_for(&base_corpus_dir, &target_contract.artifact_id);
    let corpus_config = fuzzer::corpus::Config::new(corpus_dir)
        .target_functions(target_contract.target_functions.clone())
        .max_calls(args.max_calls)
        .literals(literals);
    let corpus = fuzzer::corpus::SharedCorpus::new(corpus_config);
    let corpus_stats = corpus.load_items()?;
    info!(
        total = corpus_stats.total_count,
        parse_failed = corpus_stats.parse_failed_count,
        invalid = corpus_stats.invalid_call_count,
        valid = corpus_stats.valid_count,
        "corpus loaded"
    );

    // Initialize shared coverage and sync with corpus.
    let shared_coverage = SharedCoverage::new();
    CorpusReplayer::new(shared_coverage.clone())
        .shared_corpus(corpus.clone())
        .chain(chain.clone())
        .deployed_address(deployed_address)
        .invariant_functions(target_contract.invariant_functions.clone())
        .caller(args.deployer_address)
        .replay()?;
    info!(hit_count = shared_coverage.hit_count(), "after replay");

    // Initialize shared metrics across all fuzzer threads.
    let shared_metrics = SharedMetrics::new();

    // Initialize shared shutdown signal across all fuzzer threads.
    let shutdown_signal = Arc::new(AtomicBool::new(false));

    let fuzzers = args.threads;
    let start = std::time::Instant::now();
    let timeout = args.timeout_secs.map(std::time::Duration::from_secs);

    info!("fuzzing campaign started");

    let fuzzers_u64 = fuzzers as u64;
    let base_runs = args.max_runs / fuzzers_u64;
    let remainder = (args.max_runs % fuzzers_u64) as usize;

    let initial_config = fuzzer::Config::new()
        .chain(chain.clone())
        .target_address(deployed_address)
        .shared_corpus(corpus.clone())
        .shared_coverage(shared_coverage.clone())
        .shared_metrics(shared_metrics.clone())
        .shutdown_signal(shutdown_signal.clone())
        .invariant_functions(target_contract.invariant_functions.clone())
        .caller(args.deployer_address)
        .timeout(timeout);

    let mut handles = Vec::with_capacity(fuzzers);
    for fuzzer_id in 0..fuzzers {
        let local_max_runs = if fuzzer_id < remainder {
            base_runs + 1
        } else {
            base_runs
        };
        let seed = args.seed.wrapping_add(fuzzer_id as u64);
        // checkrs: allow(clone_in_loops)
        let mut config = initial_config.clone();
        config.max_runs = local_max_runs;
        config.seed = seed;

        let fuzzer = fuzzer::Fuzzer::new(config);
        let handle = std::thread::spawn(move || fuzzer.run());
        handles.push((fuzzer_id, handle));
    }

    let mut all_failures = Vec::new();
    for (fuzzer_id, handle) in handles {
        match handle.join() {
            Ok(Ok(result)) => {
                all_failures.extend(result.failures);
            }
            Ok(Err(e)) => {
                tracing::error!(fuzzer_id, %e, "fuzzer failed");
            }
            Err(e) => {
                tracing::error!(fuzzer_id, ?e, "fuzzer panicked");
            }
        }
    }

    let snapshot = shared_metrics.aggregate();
    let total_runs = snapshot.runs;
    let total_calls = snapshot.calls;
    let total_gas = snapshot.gas;

    info!(
        runs = total_runs,
        failures = all_failures.len(),
        "Campaign complete"
    );

    let elapsed_secs = start.elapsed().as_secs_f64();
    let calls_per_sec = if elapsed_secs > 0.0 {
        total_calls as f64 / elapsed_secs
    } else {
        0.0
    };
    let avg_gas_per_call = if total_calls > 0 {
        total_gas as f64 / total_calls as f64
    } else {
        0.0
    };

    // Report campaign result
    info!(runs = total_runs, calls = total_calls, "Fuzzing completed");
    info!(calls_per_sec = calls_per_sec, "Throughput");
    info!(avg_gas_per_call = avg_gas_per_call, "Average gas per call");
    if all_failures.is_empty() {
        info!("All invariants passed");
    } else {
        for failure in &all_failures {
            info!("");
            info!(contract = %target_contract.artifact_id.name, "[FAILED] Invariant Test");
            info!(contract = %target_contract.artifact_id.name, "Test failed after the following call sequence");
            info!("[Call Sequence]");
            info!(
                "{}",
                failure.format(&target_contract, args.deployer_address)
            );
        }
        let total = target_contract.invariant_functions.len();
        let failed = all_failures.len();
        let passed = total.saturating_sub(failed);
        info!("");
        info!(passed = passed, failed = failed, "Test summary");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use clap_verbosity_flag::Verbosity;
    use revm::primitives::U256;

    use crate::commands::fuzz::ForkModeArgs;
    use crate::evm::chain::DEFAULT_DEPLOYER;
    use crate::foundry;

    use super::Args;

    fn count_corpus_files(dir: impl AsRef<Path>) -> usize {
        let dir = dir.as_ref();
        if !dir.exists() {
            return 0;
        }
        walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension() == Some("json".as_ref()))
            .count()
    }

    fn make_args(corpus_dir: impl AsRef<Path>) -> Args {
        let corpus_dir = corpus_dir.as_ref().to_path_buf();
        Args {
            target: foundry::ArtifactId::try_from("src/L1SimpleKnob.sol:SimpleKnob").unwrap(),
            project_path: Some(PathBuf::from("fixtures/challenges")),
            deploy_value: U256::ZERO,
            deployer_address: DEFAULT_DEPLOYER,
            threads: 1,
            max_runs: 10000,
            timeout_secs: None,
            max_calls: 32,
            seed: 0,
            corpus_dir: Some(corpus_dir),
            verbosity: Verbosity::new(0, 0),
            fork_mode: ForkModeArgs::default(),
            ffi: false,
            force: false,
        }
    }

    /// Regression test: once a bug is found, the corpus must not grow on
    /// subsequent runs.
    #[test]
    fn corpus_does_not_grow_after_bug_found() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus_dir = tmp.path().join("corpus");

        // First run: the fuzzer finds the bug and adds items.
        super::run(make_args(corpus_dir.clone())).expect("first run should succeed");
        let count_after_first = count_corpus_files(&corpus_dir);
        assert!(
            count_after_first > 0,
            "corpus should have items after first run"
        );

        // Second run: the fuzzer should not add redundant items.
        super::run(make_args(corpus_dir.clone())).expect("second run should succeed");
        let count_after_second = count_corpus_files(&corpus_dir);
        assert_eq!(
            count_after_first, count_after_second,
            "corpus should not grow after bug is already found"
        );
    }
}
