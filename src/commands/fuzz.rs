//! `fuzz` CLI command implementation.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use alloy_primitives::Address;
use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use revm::primitives::{Bytes, U256};
use tracing::{debug, info, instrument};

use crate::*;

fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

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

impl ForkModeArgs {}

/// Build a [`forkdb::Config`](crate::evm::forkdb::Config) from CLI arguments.
/// Build a tree of [`DeployLibraryInput`] from the target artifact's library
/// dependencies.
///
/// Recursively resolves each dependency from `build_artifacts` and collects
/// the library initcode (including nested dependencies).
fn build_deploy_libraries(
    artifact: &foundry::Artifact,
    build_artifacts: &HashMap<foundry::ArtifactId, foundry::Artifact>,
) -> Result<Vec<evm::DeployLibraryInput>> {
    let deps = match artifact {
        foundry::Artifact::Contract(c) => c.bytecode.library_dependencies(),
        foundry::Artifact::Library(c) => c.bytecode.library_dependencies(),
        _ => return Ok(Vec::new()),
    };

    let mut libraries = Vec::new();
    for (file, names) in deps {
        for name in names {
            let identifier = format!("{}:{}", file, name);

            let temp_id = crate::foundry::ArtifactId {
                path: PathBuf::from(&file),
                name,
            };
            let lib_artifact = build_artifacts
                .get(&temp_id)
                .with_context(|| format!("library artifact missing: {}", identifier))?;

            let initcode = match lib_artifact {
                foundry::Artifact::Library(c) => c.bytecode.object.parse().unwrap_or_default(),
                _ => bail!("artifact {} is not a library", identifier),
            };

            let nested = build_deploy_libraries(lib_artifact, build_artifacts)?;
            let mut lib_input = evm::DeployLibraryInput::new(identifier, initcode);
            for nested_lib in nested {
                lib_input = lib_input.add_library(nested_lib);
            }
            libraries.push(lib_input);
        }
    }
    Ok(libraries)
}

fn build_fork_config(
    project_path: impl AsRef<Path>,
    fork_mode: &ForkModeArgs,
) -> Result<evm::forkdb::Config> {
    let cache_dir = project_path.as_ref().join("raptor").join("cache");
    let block = fork_mode
        .rpc_block
        .context("--rpc-block is required with --rpc-url")?;
    let url = fork_mode
        .rpc_url
        .as_ref()
        .context("--rpc-url is required")?;
    let config = evm::forkdb::Config::new(url.clone())
        .retries(fork_mode.rpc_retries)
        .backoff_ms(fork_mode.rpc_backoff)
        .rate_limit(fork_mode.rpc_rate_limit)
        .timeout_ms(fork_mode.rpc_timeout)
        .cache_dir(&cache_dir)
        .block_number(block);
    Ok(config)
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

    // TODO(pyk): Create ExternalLibRegistry

    // Create test chain
    info!("creating test chain");
    let mut chain_config =
        evm::chain::Config::new(&project_path).with_compiled_contracts(compiled_contracts);
    if args.fork_mode.rpc_url.is_some() {
        let fork_config = build_fork_config(&project_path, &args.fork_mode)?;
        chain_config.fork = Some(fork_config);
        info!("forking a chain"); // TODO: add chain name, block number etc
    }
    let mut chain = evm::Chain::new(chain_config)?;

    // Load target contract and prepare library dependencies.
    info!("loading target contract");
    let target_artifact = build_artifacts
        .get(&args.target)
        .context("target artifact not found")?;
    let target_contract = target::Contract::try_from(target_artifact)?;

    let libraries = build_deploy_libraries(target_artifact, &build_artifacts)?;
    if !libraries.is_empty() {
        let lib_ids: Vec<&str> = libraries.iter().map(|l| l.id.as_str()).collect();
        info!("linked external libraries: {:?}", lib_ids);
    }

    // Deploy target contract
    info!("deploying target contract");
    let mut deploy_opts = evm::DeployInput::new(target_contract.initcode.clone())
        .caller(args.deployer_address)
        .value(args.deploy_value);
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
    let literals = fuzzer::corpus::extract_literals(&build_artifacts);
    let base_corpus_dir = args
        .corpus_dir
        .unwrap_or_else(|| project_path.join("raptor").join("corpus"));
    let corpus_dir = fuzzer::corpus::get_dir(&base_corpus_dir, &target_contract.artifact_id);
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

    // Create fuzzer factory
    info!("creating fuzzer factory");
    // Create fuzzer config first so the corpus can own mutators.
    let fuzzer_config = fuzzer::Config {
        seed: args.seed,
        max_calls: args.max_calls,
    };
    let factory = fuzzer::Factory::new(
        chain,
        target_contract.clone(),
        deployed_address,
        fuzzer_config,
        corpus,
    )
    .with_caller(args.deployer_address);

    let fuzzers = args.threads;
    let start = std::time::Instant::now();
    let timeout = args.timeout_secs.map(std::time::Duration::from_secs);

    info!("Fuzzing campaign started");

    let fuzzers_u64 = fuzzers as u64;
    let base_runs = args.max_runs / fuzzers_u64;
    let remainder = (args.max_runs % fuzzers_u64) as usize;

    // Progress reporting thread.
    let progress_shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let metrics = factory.metrics().clone();
    let corpus = factory.corpus().clone();
    let progress_handle = {
        let shutdown = Arc::clone(&progress_shutdown);
        std::thread::spawn(move || {
            let mut last_calls = 0u64;
            let mut last_gas = 0u64;
            let mut last_time = std::time::Instant::now();
            while !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(start);
                let interval_secs = now.duration_since(last_time).as_secs_f64().max(1e-6);

                let snapshot = metrics.aggregate();
                let calls_delta = snapshot.calls.saturating_sub(last_calls);
                let gas_delta = snapshot.gas.saturating_sub(last_gas);
                let calls_per_sec = (calls_delta as f64 / interval_secs) as u64;
                let gas_per_sec = (gas_delta as f64 / interval_secs) as u64;
                let elapsed_str = format_duration(elapsed);
                let calls_str = format!("{}({}/s)", snapshot.calls, calls_per_sec);
                let corpus_stats = corpus.stats();

                info!(
                    elapsed = %elapsed_str,
                    runs = snapshot.runs,
                    calls = %calls_str,
                    corpus = corpus_stats.item_count,
                    failures = snapshot.failures,
                    gas_per_sec = gas_per_sec,
                    "fuzz:"
                );

                last_calls = snapshot.calls;
                last_gas = snapshot.gas;
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
        let mut fuzzer = factory.create(fuzzer_id);
        let handle = std::thread::spawn(move || fuzzer.run(local_max_runs, timeout));
        handles.push((fuzzer_id, handle));
    }

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
            }
            Ok(Err(e)) => {
                tracing::error!(fuzzer_id, %e, "fuzzer failed");
            }
            Err(e) => {
                tracing::error!(fuzzer_id, ?e, "fuzzer panicked");
            }
        }
    }

    progress_shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = progress_handle.join();

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
            info!(contract = %target_contract.artifact_id.name, test = %failure.function_name, "[FAILED] Invariant Test");
            info!(contract = %target_contract.artifact_id.name, test = %failure.function_name, "Test failed after the following call sequence");
            info!("[Call Sequence]");
            info!(
                "{}",
                crate::fuzzer::format_failure(&target_contract, failure, args.deployer_address)
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
