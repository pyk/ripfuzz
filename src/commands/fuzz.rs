//! `fuzz` CLI command implementation.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use alloy_primitives::Address;
use anyhow::{Context, Result};
use clap::Parser;
use revm::primitives::{Bytes, U256};
use tracing::{debug, instrument};

use crate::console::Console;
use crate::corpus::{
    Call, CorpusConfig, CorpusReplayer, ExtractedLiterals, Item, SharedCorpus,
    SharedFailedCorpusItem,
};
use crate::evm::{
    Chain, ChainConfig, Contract, CoverageContext, CoverageReporter, DeployInput, ForkDBConfig,
    SetupInput, SharedCoverage, Trace, TraceContext, Transaction,
};
use crate::formatter;
use crate::foundry::{Artifact, ArtifactId, BuildOptions, Project};
use crate::fuzzer::{Fuzzer, FuzzerConfig, SharedMetrics};
use crate::logger;
use crate::shrinker::{Shrinker, ShrinkerConfig};

#[derive(Debug, Parser)]
pub struct Args {
    /// Target contract identifier (e.g. ./test/Contract.sol:Contract).
    #[arg(value_name = "TARGET")]
    pub target: ArtifactId,

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
    #[arg(long = "deploy-value", default_value = "0", value_parser = Args::parse_balance, value_name = "WEI", help_heading = "Project & Deployment")]
    pub deploy_value: U256,

    /// Account address used to deploy the target contract.
    #[arg(
        long = "deployer",
        default_value_t = crate::evm::DEFAULT_DEPLOYER,
        value_parser = Args::parse_address,
        value_name = "ADDRESS",
        help_heading = "Project & Deployment"
    )]
    pub deployer_address: Address,

    // Campaign Limits
    /// Number of parallel fuzzer threads to spawn.
    #[arg(short = 'w', long = "threads", default_value_t = Args::default_threads(), value_parser = Args::parse_threads, value_name = "N", help_heading = "Campaign Limits")]
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

    // Shrinker
    /// Maximum number of shrink runs across all shrinker threads.
    #[arg(
        long = "shrink-runs",
        default_value = "10000",
        value_name = "N",
        help_heading = "Shrinker"
    )]
    pub shrink_runs: u64,

    /// Timeout in seconds for the shrinking phase.
    #[arg(
        long = "shrink-timeout",
        value_name = "SECS",
        help_heading = "Shrinker"
    )]
    pub shrink_timeout_secs: Option<u64>,

    /// Number of parallel shrinker threads to spawn.
    #[arg(
        long = "shrink-threads",
        value_parser = Args::parse_threads,
        value_name = "N",
        help_heading = "Shrinker"
    )]
    pub shrink_threads: Option<usize>,

    // Corpus
    /// Directory to load and persist coverage-guided corpus files.
    #[arg(long = "corpus-dir", value_name = "DIR", help_heading = "Corpus")]
    pub corpus_dir: Option<PathBuf>,

    // Logging
    /// Log verbosity level.
    #[arg(
        long = "log-level",
        default_value = "info",
        value_name = "LEVEL",
        help_heading = "Logging"
    )]
    pub log_level: tracing::Level,

    /// Disable writing log output to a file.
    #[arg(long = "disable-log", help_heading = "Logging")]
    pub disable_log: bool,

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

    /// Treat any transaction revert as a failed assertion.
    #[arg(long = "fail-on-revert", help_heading = "Fuzzing Parameters")]
    pub fail_on_revert: bool,
}

impl Args {
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
            return U256::from_str_radix(stripped, 16)
                .map_err(|e| format!("invalid hex balance: {e}"));
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
    /// Build a [`ForkDBConfig`](crate::evm::ForkDBConfig) from CLI arguments.
    pub fn build_fork_config(&self, project_path: impl AsRef<Path>) -> Result<ForkDBConfig> {
        let cache_dir = project_path.as_ref().join("raptor").join("cache");
        let block = self
            .rpc_block
            .context("--rpc-block is required with --rpc-url")?;
        let url = self.rpc_url.as_ref().context("--rpc-url is required")?;
        let config = ForkDBConfig::new(url.clone())
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
    let mut console = Console::new();
    console.set_disabled(args.disable_log);
    console.print(format!("starting raptor v{}", env!("CARGO_PKG_VERSION")))?;

    // Resolve project path
    let project_path = args.project_path.map(Ok).unwrap_or_else(env::current_dir)?;

    // Generate campaign ID for coverage report, trace output, and log file.
    let now = jiff::Zoned::now();
    let date = jiff::fmt::strtime::format("%Y-%m-%d", &now).unwrap_or_default();
    let hour = jiff::fmt::strtime::format("%H%M", &now).unwrap_or_default();
    let uuid = uuid::Uuid::new_v4();
    let uuid_str: String = uuid.into();
    let uuid_prefix = uuid_str.split('-').next().unwrap_or_default();
    let campaign_id = format!("{date}-{hour}-{uuid_prefix}");

    if !args.disable_log {
        let log_file = project_path
            .join("raptor")
            .join("campaigns")
            .join(&campaign_id)
            .join("fuzz.log");
        logger::init(&log_file, args.log_level)?;
    }

    debug!(?project_path, "resolved project path");

    // Build project
    console.begin("building foundry project ...")?;
    let project = Project::new(&project_path);
    let build_opts = BuildOptions::new().force(args.force);
    if let Err(e) = project.build(build_opts) {
        console.end_fail("building foundry project failed")?;
        console.print_line(format!("{e:#}"))?;
        return Err(e);
    }
    console.update("built foundry project")?;
    console.end()?;

    // Load build artifacts
    console.begin("loading build artifacts ...")?;
    let build_artifacts = match project.load_artifacts() {
        Ok(artifacts) => artifacts,
        Err(e) => {
            console.end_fail("loading build artifacts failed")?;
            console.print_line(format!("{e:#}"))?;
            return Err(e);
        }
    };
    console.update(format!(
        "loaded {} build artifacts",
        formatter::num(build_artifacts.len() as u64)
    ))?;
    console.end()?;

    // Load target contract and prepare library dependencies.
    console.begin(format!("loading target contract {} ...", args.target.name))?;
    let target_contract = match Contract::try_get(&build_artifacts, &args.target) {
        Ok(c) => c,
        Err(e) => {
            console.end_fail(format!(
                "loading target contract {} failed",
                args.target.name
            ))?;
            console.print_line(format!("{e:#}"))?;
            return Err(e);
        }
    };
    console.update(format!("loaded {} as target contract", args.target.name))?;
    console.end()?;

    // TODO(pyk): Create InitcodeRegistry
    // Build compiled-contract registry for vm.getCode
    let mut compiled_contracts = HashMap::new();
    for (id, artifact) in &build_artifacts {
        let bytecode = match artifact {
            Artifact::Contract(c) => &c.bytecode.object,
            Artifact::Library(c) => &c.bytecode.object,
            _ => continue,
        };
        let initcode: Bytes = bytecode.parse().unwrap_or_default();
        if initcode.is_empty() {
            continue;
        }
        compiled_contracts.insert(id.into(), initcode);
    }

    // Create test chain
    console.begin("spawning test chain ...")?;
    let mut chain_config = ChainConfig::new(&project_path)
        .with_compiled_contracts(compiled_contracts)
        .coverage(true);
    if args.fork_mode.rpc_url.is_some() {
        console.update("forking chain")?;
        let fork_config = match args.fork_mode.build_fork_config(&project_path) {
            Ok(c) => c,
            Err(e) => {
                console.end_fail("forking chain failed")?;
                console.print_line(format!("{e:#}"))?;
                return Err(e);
            }
        };
        chain_config = chain_config.fork(fork_config);
    }
    let mut chain = match Chain::new(chain_config) {
        Ok(c) => c,
        Err(e) => {
            console.end_fail("spawning test chain failed")?;
            console.print_line(format!("{e:#}"))?;
            return Err(e);
        }
    };
    console.update("spawned test chain")?;
    console.end()?;
    console.print_line(format!(
        "    chain id        : {}\n    evm version     : {}\n    block number    : #{}\n    block timestamp : {}",
        chain.cfg_env().chain_id,
        chain.cfg_env().spec.to_string().to_lowercase(),
        chain.block_env().number,
        chain.block_env().timestamp,
    ))?;

    // Deploy target contract
    let contract_name = &target_contract.artifact_id.name;
    console.begin(format!("deploying {contract_name}..."))?;
    let mut deploy_opts = DeployInput::new(&target_contract.initcode)
        .caller(args.deployer_address)
        .value(args.deploy_value);
    let libraries = target_contract.libraries.clone();
    for lib in libraries {
        deploy_opts = deploy_opts.add_library(lib);
    }
    let deployment = chain.deploy(deploy_opts)?;
    if !deployment.result.success {
        let mut ctx = TraceContext::from_project(&project)?;
        if let Some(addr) = deployment.trace.roots.first().and_then(|r| r.address) {
            ctx = ctx.with_label(addr, contract_name);
        }
        for (addr, label) in chain.labels() {
            ctx = ctx.with_label(*addr, label);
        }
        let trace_dir = project_path
            .join("raptor")
            .join("campaigns")
            .join(&campaign_id);
        fs::create_dir_all(&trace_dir)?;
        let trace_file = trace_dir.join("trace.log");
        let trace = deployment.trace.display_with(&ctx);
        fs::write(&trace_file, format!("{trace}"))?;
        console.end_fail(format!("failed to deploy {contract_name}"))?;
        console.print_line(format!("    trace: {}", trace_file.display()))?;
        return Err(anyhow::anyhow!("target contract deployment failed"));
    }
    let deployed_address = deployment
        .address
        .context("deployment succeeded but created_address is missing")?;
    console.update(format!("deployed {contract_name}"))?;
    console.end()?;

    let contract_size = deployment
        .result
        .output
        .as_ref()
        .map(|b| b.len())
        .unwrap_or(0);
    console.print_line(format!(
        "    {:16} : {}\n    {:16} : {}\n    {:16} : {}\n    {:16} : {}",
        "deployer",
        args.deployer_address,
        "msg value",
        formatter::eth(args.deploy_value),
        "contract address",
        deployed_address,
        "contract size",
        formatter::kb(contract_size),
    ))?;

    // Run setup if present
    if let Some(ref setup) = target_contract.setup_function {
        console.begin("calling setup")?;
        let setup_output = match chain.setup(
            SetupInput::new(deployed_address)
                .calldata(Bytes::from(setup.selector().as_slice().to_vec()))
                .caller(args.deployer_address),
        ) {
            Ok(output) => output,
            Err(e) => {
                console.end_fail("calling setup failed")?;
                console.print_line(format!("{e:#}"))?;
                return Err(e);
            }
        };
        if !setup_output.result.success {
            let mut ctx =
                TraceContext::from_project(&project)?.with_label(deployed_address, contract_name);
            for (addr, label) in chain.labels() {
                ctx = ctx.with_label(*addr, label);
            }
            let trace_dir = project_path
                .join("raptor")
                .join("campaigns")
                .join(&campaign_id);
            fs::create_dir_all(&trace_dir)?;
            let trace_file = trace_dir.join("trace.log");
            let trace = setup_output.trace.display_with(&ctx);
            fs::write(&trace_file, format!("{trace}"))?;
            console.end_fail("failed to call setup")?;
            console.print_line(format!("    trace: {}", trace_file.display()))?;
            return Err(anyhow::anyhow!("setup failed"));
        }
        console.end()?;
    }

    // Extract literals from build artifacts so the fuzzer can seed random value
    // generation with concrete values found across the entire project.
    let literals = ExtractedLiterals::from_artifacts(&build_artifacts);
    let base_corpus_dir = args
        .corpus_dir
        .unwrap_or_else(|| project_path.join("raptor").join("corpus"));
    let corpus_dir = SharedCorpus::dir_for(&base_corpus_dir, &target_contract.artifact_id);
    let corpus_config = CorpusConfig::new(corpus_dir)
        .target_functions(target_contract.target_functions.clone())
        .max_calls(args.max_calls)
        .literals(literals.clone());
    let corpus = SharedCorpus::new(corpus_config);
    let corpus_stats = corpus.load_items()?;

    if corpus_stats.total_count > 0 {
        console.begin("loading corpus items ...")?;
        console.update(format!(
            "loaded {} corpus items",
            formatter::num(corpus_stats.valid_count as u64)
        ))?;
        console.end()?;
        console.print_line(format!(
            "    {:8}: {} items\n    {:8}: {} items\n    {:8}: {} items",
            "on disk",
            formatter::num(corpus_stats.total_count as u64),
            "valid",
            formatter::num(corpus_stats.valid_count as u64),
            "invalid",
            formatter::num(
                (corpus_stats.parse_failed_count + corpus_stats.invalid_call_count) as u64
            )
        ))?;
    }

    // Initialize shared coverage and sync with corpus.
    let shared_coverage = SharedCoverage::new();
    let replay_count = corpus_stats.valid_count;

    if replay_count > 0 {
        console.begin(format!("replaying {replay_count} corpus items ..."))?;
        if let Err(e) = CorpusReplayer::new(shared_coverage.clone())
            .shared_corpus(corpus.clone())
            .chain(chain.clone())
            .deployed_address(deployed_address)
            .invariant_functions(target_contract.invariant_functions.clone())
            .caller(args.deployer_address)
            .replay()
        {
            console.end_fail("replaying corpus items failed")?;
            console.print_line(format!("{e:#}"))?;
            return Err(e);
        }
        console.update(format!("replayed {replay_count} corpus items"))?;
        console.end()?;
        console.print_line(format!(
            "    {:16} : {}\n    {:16} : {}\n    {:16} : {}\n    {:16} : {}\n    {:16} : {}",
            "unique contracts",
            formatter::num(shared_coverage.contract_count() as u64),
            "total edges",
            formatter::num(shared_coverage.edge_count() as u64),
            "total depths",
            formatter::num(shared_coverage.depth_count() as u64),
            "total reverts",
            formatter::num(shared_coverage.revert_count() as u64),
            "total jumps",
            formatter::num(shared_coverage.jump_count() as u64)
        ))?;
    }

    // Initialize shared metrics across all fuzzer threads.
    let all_function_signatures: Vec<String> = target_contract
        .target_functions
        .iter()
        .chain(target_contract.invariant_functions.iter())
        .map(|f| f.signature())
        .collect();
    let shared_metrics = SharedMetrics::new(all_function_signatures.clone());

    // Initialize shared shutdown signal across all fuzzer threads.
    let shutdown_signal = Arc::new(AtomicBool::new(false));

    let fuzzers = args.threads;
    let timeout = args.timeout_secs.map(std::time::Duration::from_secs);

    let fuzzers_u64 = fuzzers as u64;
    let base_runs = args.max_runs / fuzzers_u64;
    let remainder = (args.max_runs % fuzzers_u64) as usize;

    let initial_config = FuzzerConfig::new()
        .chain(chain.clone())
        .target_address(deployed_address)
        .shared_corpus(corpus.clone())
        .shared_coverage(shared_coverage.clone())
        .shared_metrics(shared_metrics.clone())
        .shutdown_signal(shutdown_signal.clone())
        .invariant_functions(target_contract.invariant_functions.clone())
        .caller(args.deployer_address)
        .timeout(timeout)
        .fail_on_revert(args.fail_on_revert);

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

        let fuzzer = Fuzzer::new(config);
        let handle = std::thread::spawn(move || fuzzer.run());
        handles.push((fuzzer_id, handle));
    }

    let contract_name = &target_contract.artifact_id.name;
    console.begin(format!(
        "fuzzing {contract_name} with {fuzzers} threads ..."
    ))?;
    console.new_line()?;

    // Print initial stats immediately so the user sees the dashboard
    // right after the title line, then refresh every 100ms.
    let stats_ctx = formatter::CampaignStats::new(
        &shared_coverage,
        &corpus,
        &target_contract.target_functions,
        &target_contract.invariant_functions,
    );
    let mut snapshot = shared_metrics.aggregate();
    let mut function_metrics = shared_metrics.function_metrics();
    let mut stats = stats_ctx.format(&snapshot, &function_metrics);
    console.print_clearable(stats)?;
    let mut last_print = std::time::Instant::now();

    while handles.iter().any(|(_, h)| !h.is_finished()) {
        snapshot = shared_metrics.aggregate();
        if last_print.elapsed().as_millis() >= 100 {
            function_metrics = shared_metrics.function_metrics();
            stats = stats_ctx.format(&snapshot, &function_metrics);
            console.print_clearable(stats)?;
            last_print = std::time::Instant::now();
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
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

    if all_failures.is_empty() {
        console.set_message(format!("fuzzed {contract_name} with {fuzzers} threads"));
        console.clear_and_end()?;
        let function_metrics = shared_metrics.function_metrics();
        let stats = stats_ctx.format(&shared_metrics.aggregate(), &function_metrics);
        console.print_line(stats)?;
        console.new_line()?;

        console.begin("generating coverage reports ...")?;
        match write_coverage_report(&project, &campaign_id, &shared_coverage, &target_contract) {
            Ok(files) => {
                let count = files.len();
                console.update(format!("generated {count} coverage reports"))?;
                console.end()?;
                for (file, pct) in files {
                    console.print_line(format!("    [{pct:.2}%] {}", file.display()))?;
                }
            }
            Err(e) => {
                console.end_fail("failed to generate coverage reports")?;
                tracing::error!(%e, "failed to generate coverage reports");
            }
        }

        console.print("no failed assertions found!")?;
        console.print("raptor out. see ya")?;

        return Ok(());
    }

    let smallest_failure = all_failures
        .iter()
        .min_by_key(|f| f.item.calls.len())
        .context("no failures found")?;

    let initial_calls = smallest_failure.item.calls.len();
    console.set_message(format!("fuzzed {contract_name} with {fuzzers} threads"));
    console.clear_and_end()?;
    let function_metrics = shared_metrics.function_metrics();
    let stats = stats_ctx.format(&shared_metrics.aggregate(), &function_metrics);
    console.print_line(stats)?;
    console.new_line()?;
    console.print_fail(format!(
        "found failed assertions in {} corpus items",
        all_failures.len()
    ))?;

    // Combine the smallest failing item with invariants so the shrinker
    // operates on a single corpus item and never appends invariants.
    let mut combined_calls = smallest_failure.item.calls.clone();
    let invariant_calls: Vec<Call> = target_contract
        .invariant_functions
        .iter()
        // checkrs: allow(clone_in_iterator)
        .map(|func| Call {
            function: func.clone(),
            args: alloy_dyn_abi::DynSolValue::Tuple(vec![]),
            value: None,
            caller: args.deployer_address,
        })
        .collect();
    combined_calls.extend(invariant_calls);
    let combined_item = Item::from(combined_calls);

    // Include both target and invariant functions so the shrinker can
    // generate replacement calls for any position in the sequence.
    let all_functions: Vec<alloy_json_abi::Function> = target_contract
        .target_functions
        .iter()
        .chain(target_contract.invariant_functions.iter())
        .cloned()
        .collect();

    let failed_corpus_config = CorpusConfig::new(PathBuf::new())
        .target_functions(all_functions)
        .max_calls(args.max_calls)
        .literals(literals);
    let shared_failed_item = SharedFailedCorpusItem::new(combined_item, failed_corpus_config);

    // Spawn shrinker threads.
    let shrink_threads = args.shrink_threads.unwrap_or(args.threads);
    let shrink_timeout = args.shrink_timeout_secs.map(std::time::Duration::from_secs);
    let shrinker_shutdown = Arc::new(AtomicBool::new(false));
    let shrinker_metrics = SharedMetrics::new(all_function_signatures);

    let shrinkers_u64 = shrink_threads as u64;
    let base_shrink_runs = args.shrink_runs / shrinkers_u64;
    let shrink_remainder = (args.shrink_runs % shrinkers_u64) as usize;

    let mut shrinker_handles = Vec::with_capacity(shrink_threads);
    for shrinker_id in 0..shrink_threads {
        let local_max_runs = if shrinker_id < shrink_remainder {
            base_shrink_runs + 1
        } else {
            base_shrink_runs
        };
        let seed = args
            .seed
            .wrapping_add(shrinker_id as u64)
            .wrapping_add(1000);
        // checkrs: allow(clone_in_loops)
        let shrinker_chain = chain.clone();
        // checkrs: allow(clone_in_loops)
        let shrinker_shared_item = shared_failed_item.clone();
        // checkrs: allow(clone_in_loops)
        let shrinker_shutdown = shrinker_shutdown.clone();
        let shrinker_config = ShrinkerConfig::new()
            .chain(shrinker_chain)
            .target_address(deployed_address)
            .shared_failed_item(shrinker_shared_item)
            .shutdown_signal(shrinker_shutdown)
            .max_runs(local_max_runs)
            .timeout(shrink_timeout)
            .seed(seed)
            // checkrs: allow(clone_in_loops)
            .shared_metrics(shrinker_metrics.clone())
            .fail_on_revert(args.fail_on_revert);
        let shrinker = Shrinker::new(shrinker_config);
        let handle = std::thread::spawn(move || shrinker.run());
        shrinker_handles.push(handle);
    }

    console.begin(format!(
        "shrinking {} calls with {} threads",
        formatter::num(initial_calls as u64),
        formatter::num(shrink_threads as u64)
    ))?;
    while shrinker_handles.iter().any(|h| !h.is_finished()) {
        let snapshot = shrinker_metrics.aggregate();
        let elapsed_secs = snapshot.elapsed.as_secs_f64();
        let calls_per_sec = if elapsed_secs > 0.0 {
            (snapshot.calls as f64 / elapsed_secs) as u64
        } else {
            0
        };
        let gas_per_sec = if elapsed_secs > 0.0 {
            (snapshot.gas as f64 / elapsed_secs) as u64
        } else {
            0
        };
        console.update_with_elapsed(
            format!(
                "shrinking: {} threads {} runs {} calls/s {} gas/s",
                formatter::num(shrink_threads as u64),
                formatter::num(snapshot.runs),
                formatter::num(calls_per_sec),
                formatter::num(gas_per_sec),
            ),
            elapsed_secs,
        )?;
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    for handle in shrinker_handles {
        match handle.join() {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                tracing::error!(%e, "shrinker failed");
            }
            Err(e) => {
                tracing::error!(?e, "shrinker panicked");
            }
        }
    }

    // Retrieve the smallest item found by the shrinkers.
    let shrunk_item = shared_failed_item.item();
    let shrunk_calls = shrunk_item.calls.len();
    let shrunk_call_word = if shrunk_calls == 1 { "call" } else { "calls" };
    console.set_message(format!(
        "shrank {} calls to {} {} with {} threads",
        formatter::num(initial_calls as u64),
        formatter::num(shrunk_calls as u64),
        shrunk_call_word,
        formatter::num(shrink_threads as u64)
    ));
    console.end()?;

    // Re-run the shrunk item with the chain tracer enabled.
    let mut trace_chain = chain.clone();
    trace_chain.set_trace(true);

    let transactions: Vec<Transaction> = shrunk_item
        .calls
        .iter()
        .map(|call| call.into_transaction(deployed_address))
        .collect();

    let exec = trace_chain.exec(&transactions)?;

    // TODO(pyk): assert that trace should exists; do not use if else
    if let Some(trace) = exec.trace {
        console.begin("writing trace file ...")?;
        let trace_file = match write_trace_to_file(
            &trace,
            &project,
            &project_path,
            &campaign_id,
            deployed_address,
            contract_name,
            &chain,
        ) {
            Ok(f) => f,
            Err(e) => {
                console.end_fail("writing trace file failed")?;
                console.print_line(format!("{e:#}"))?;
                return Err(e);
            }
        };
        console.update(format!("trace: {}", trace_file.display()))?;
        console.end()?;
    }

    console.begin("generating coverage reports ...")?;
    match write_coverage_report(&project, &campaign_id, &shared_coverage, &target_contract) {
        Ok(files) => {
            let count = files.len();
            console.update(format!("generated {count} coverage reports"))?;
            console.end()?;
            for (file, pct) in files {
                console.print_line(format!("    [{pct:.2}%] {}", file.display()))?;
            }
        }
        Err(e) => {
            console.end_fail("failed to generate coverage reports")?;
            tracing::error!(%e, "failed to generate coverage reports");
        }
    }

    Ok(())
}

fn write_coverage_report(
    project: &Project,
    campaign_id: &str,
    shared_coverage: &SharedCoverage,
    target_contract: &Contract,
) -> Result<Vec<(PathBuf, f64)>> {
    let context = CoverageContext::from_project(project)?
        .with_target_artifact(&target_contract.artifact_id)?;

    let reporter = CoverageReporter::new()
        .coverage(shared_coverage.clone())
        .target_functions(
            target_contract
                .target_functions
                .iter()
                .chain(target_contract.invariant_functions.iter())
                .cloned()
                .collect(),
        )
        .context(context);

    let coverage_dir = project
        .path
        .join("raptor")
        .join("campaigns")
        .join(campaign_id)
        .join("coverage");
    fs::create_dir_all(&coverage_dir)?;

    let mut generated = Vec::new();
    for (signature, func_report) in reporter.get_reports() {
        let name = signature.split('(').next().unwrap_or(&signature);
        let func_file = coverage_dir.join(format!(
            "{}.txt",
            name.replace(|c: char| !c.is_alphanumeric(), "_")
        ));
        fs::write(&func_file, format!("{func_report}"))?;
        let relative_path = func_file
            .strip_prefix(&project.path)
            .unwrap_or(&func_file)
            .to_path_buf();
        generated.push((relative_path, func_report.coverage));
    }

    Ok(generated)
}

fn write_trace_to_file(
    trace: &Trace,
    project: &Project,
    project_path: impl AsRef<Path>,
    campaign_id: &str,
    deployed_address: Address,
    contract_name: &str,
    chain: &Chain,
) -> Result<PathBuf> {
    let mut ctx = TraceContext::from_project(project)?.with_label(deployed_address, contract_name);
    for (addr, label) in chain.labels() {
        ctx = ctx.with_label(*addr, label);
    }
    let trace_dir = project_path
        .as_ref()
        .join("raptor")
        .join("campaigns")
        .join(campaign_id);
    fs::create_dir_all(&trace_dir)?;
    let trace_file = trace_dir.join("trace.log");
    let trace_str = trace.display_with(&ctx);
    fs::write(&trace_file, format!("{trace_str}"))?;
    Ok(trace_file)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use revm::primitives::U256;

    use crate::evm::DEFAULT_DEPLOYER;
    use crate::foundry;

    use super::*;

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
            log_level: tracing::Level::INFO,
            disable_log: true,
            fork_mode: ForkModeArgs::default(),
            ffi: false,
            force: false,
            fail_on_revert: false,
            shrink_runs: 1,
            shrink_timeout_secs: None,
            shrink_threads: None,
        }
    }

    /// Regression test: once a bug is found, the corpus must not grow on
    /// subsequent runs.
    #[test]
    fn corpus_does_not_grow_after_bug_found() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus_dir = tmp.path().join("corpus");

        // First run: the fuzzer finds the bug and adds items.
        run(make_args(corpus_dir.clone())).expect("first run should succeed");
        let count_after_first = count_corpus_files(&corpus_dir);
        assert!(
            count_after_first > 0,
            "corpus should have items after first run"
        );

        // Second run: the fuzzer should not add redundant items.
        run(make_args(corpus_dir.clone())).expect("second run should succeed");
        let count_after_second = count_corpus_files(&corpus_dir);
        assert_eq!(
            count_after_first, count_after_second,
            "corpus should not grow after bug is already found"
        );
    }
}
