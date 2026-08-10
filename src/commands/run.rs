//! `run` CLI command implementation.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use alloy_primitives::Address;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use revm::primitives::{Bytes, U256};
use tracing::{debug, instrument};

use crate::console::Console;
use crate::corpus::{
    Call, CorpusConfig, CorpusReplayer, ExtractedLiterals, Item, SharedCorpus,
    SharedFailedCorpusItem,
};
use crate::evm::{
    Chain, ChainConfig, Contract, CoverageReporter, DeployInput, ForkDBConfig, SetupInput,
    SharedCoverage, Trace, TraceContext, Transaction,
};
use crate::formatter;
use crate::foundry::{Artifact, ArtifactId, BuildOptions, Project};
use crate::fuzzer::{Fuzzer, FuzzerConfig, SharedFailedAssertions, SharedMetrics};
use crate::logger;
use crate::max::{
    MaxFuzzer, MaxFuzzerConfig, MaxFuzzerCorpus, MaxObjective, MaxResult, MaxShrinker,
    MaxShrinkerConfig, MaxShrinkerCorpus,
};
use crate::shrinker::{Shrinker, ShrinkerConfig};

#[derive(Debug, Parser)]
pub struct Args {
    /// Harness contract identifier: bare name (`Harness`) or full artifact id
    /// (`src/Harness.sol:Harness`).
    #[arg(value_name = "HARNESS")]
    pub harness: String,

    // Project & Deployment
    /// Path to the Foundry project root.
    #[arg(
        short = 'p',
        long = "project",
        value_name = "PATH",
        help_heading = "Project & Deployment"
    )]
    pub project_path: Option<PathBuf>,

    /// Wei to send during harness contract deployment.
    #[arg(long = "deploy-value", default_value = "0", value_parser = Args::parse_balance, value_name = "WEI", help_heading = "Project & Deployment")]
    pub deploy_value: U256,

    /// Account address used to deploy the harness contract.
    #[arg(
        long = "deployer",
        default_value_t = crate::evm::DEFAULT_DEPLOYER,
        value_parser = Args::parse_address,
        value_name = "ADDRESS",
        help_heading = "Project & Deployment"
    )]
    pub deployer_address: Address,

    // Campaign Limits
    /// Gas limit for each fuzzer-generated transaction.
    #[arg(
        long = "gas-limit",
        default_value = "12500000",
        value_name = "GAS",
        help_heading = "Campaign Limits"
    )]
    pub gas_limit: u64,

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

    /// Maximum number of distinct failed assertions to collect before stopping
    /// the fuzzing campaign.
    #[arg(
        long = "max-failures",
        default_value = "1",
        value_parser = Args::parse_max_failures,
        value_name = "N",
        help_heading = "Campaign Limits"
    )]
    pub max_failures: usize,

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
        default_value = "100",
        value_name = "N",
        help_heading = "Fuzzing Parameters"
    )]
    pub max_calls: usize,

    /// Random seed for reproducibility.
    ///
    /// When not provided, a random seed is generated and printed at campaign
    /// start so the run can be reproduced later.
    #[arg(long = "seed", value_name = "N", help_heading = "Fuzzing Parameters")]
    pub seed: Option<u64>,

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

    /// Run in max mode: maximize `max_*` functions instead of checking
    /// invariants. Invariant mode and max mode are mutually exclusive.
    #[arg(long = "max-mode", help_heading = "Fuzzing Parameters")]
    pub max_mode: bool,

    /// Additional Foundry projects whose build artifacts are loaded for
    /// coverage and trace resolution.
    ///
    /// Useful in fork mode when the harness contract interacts with
    /// contracts compiled in separate projects. Each path must point to a
    /// Foundry project root that contains an `out/` directory with compiled
    /// artifacts (run `forge build --ast --extra-output storageLayout` there
    /// first).
    ///
    /// Artifacts from these projects are merged into the coverage reporter so
    /// that on-chain bytecodes executed during fork mode can be matched back
    /// to their source maps and source files.
    #[arg(
        long = "external-project",
        value_name = "PATH",
        help_heading = "Project & Deployment"
    )]
    pub external_projects: Vec<PathBuf>,
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

    fn parse_max_failures(s: &str) -> Result<usize, String> {
        let n = s
            .parse::<usize>()
            .map_err(|e| format!("invalid max-failures value: {e}"))?;
        if n == 0 {
            return Err("max-failures must be at least 1".into());
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

/// Returns the ripfuzz data directory for the given project path.
fn ripfuzz_dir(project_path: impl AsRef<Path>) -> PathBuf {
    project_path.as_ref().join(".ripfuzz")
}

/// Load `.env` from `dir` into the process environment when the file exists.
///
/// Existing environment variables are preserved (not overridden). Returns the
/// loaded path, or `None` when no `.env` file is present.
fn load_dotenv(dir: impl AsRef<Path>) -> Result<Option<PathBuf>> {
    let path = dir.as_ref().join(".env");
    match dotenvy::from_path(&path) {
        Ok(()) => Ok(Some(path)),
        Err(e) if e.not_found() => Ok(None),
        Err(e) => Err(e).with_context(|| format!("failed to load {}", path.display())),
    }
}

#[instrument(skip(args), fields(harness = ?args.harness, threads = args.threads, max_runs = args.max_runs))]
pub fn run(args: Args) -> Result<()> {
    let mut console = Console::new();
    console.set_disabled(args.disable_log);
    // Resolve the campaign seed early so it can be logged before any work.
    let campaign_seed = match args.seed {
        Some(s) => {
            console.print(format!(
                "starting ripfuzz v{} (seed: {s}, user-provided)",
                env!("CARGO_PKG_VERSION")
            ))?;
            s
        }
        None => {
            let s = fastrand::Rng::new().u64(1..=100_000);
            console.print(format!(
                "starting ripfuzz v{} (seed: {s})",
                env!("CARGO_PKG_VERSION")
            ))?;
            s
        }
    };

    // Resolve project path
    let project_path = args
        .project_path
        .clone()
        .map(Ok)
        .unwrap_or_else(env::current_dir)?;

    // Load project `.env` so `vm.getEnv` can read those values.
    let dotenv_path = load_dotenv(&project_path)?;

    // Generate campaign ID for coverage report, trace output, and log file.
    let now = jiff::Zoned::now();
    let timestamp = jiff::fmt::strtime::format("%Y-%m-%d-%H%M%S", &now).unwrap_or_default();
    let uuid = uuid::Uuid::new_v4();
    let uuid_str: String = uuid.into();
    let uuid_prefix = uuid_str.split('-').next().unwrap_or_default();
    let campaign_id = format!("{timestamp}-{uuid_prefix}");

    if !args.disable_log {
        let log_file = ripfuzz_dir(&project_path)
            .join("campaigns")
            .join(&campaign_id)
            .join("fuzz.log");
        logger::init(&log_file, args.log_level)?;
    }

    debug!(?project_path, "resolved project path");
    if let Some(ref path) = dotenv_path {
        debug!(?path, "loaded environment from .env");
    }

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

    // Load external project artifacts for coverage and trace resolution.
    let mut external_artifacts = Vec::new();
    for ext_path in &args.external_projects {
        let ext_project = Project::new(ext_path);
        match ext_project.load_artifacts() {
            Ok(artifacts) => {
                console.print_line(format!(
                    "    loaded {} artifacts from external project {}",
                    formatter::num(artifacts.len() as u64),
                    ext_path.display()
                ))?;
                for (_, mut artifact) in artifacts {
                    artifact.set_project_path(ext_path);
                    external_artifacts.push(artifact);
                }
            }
            Err(e) => {
                console.print_line(format!(
                    "    warning: failed to load artifacts from {}: {e:#}",
                    ext_path.display()
                ))?;
            }
        }
    }

    // Resolve the harness (bare name or full artifact id) then load it.
    console.begin(format!("loading harness contract {} ...", args.harness))?;
    let harness_id = match ArtifactId::resolve(&args.harness, &build_artifacts) {
        Ok(id) => id,
        Err(e) => {
            console.end_fail(format!("loading harness contract {} failed", args.harness))?;
            console.print_line(format!("{e:#}"))?;
            return Err(e);
        }
    };
    let harness_contract = match Contract::try_get(&build_artifacts, &harness_id) {
        Ok(c) => c,
        Err(e) => {
            console.end_fail(format!(
                "loading harness contract {} failed",
                harness_id.name
            ))?;
            console.print_line(format!("{e:#}"))?;
            return Err(e);
        }
    };
    console.update(format!("loaded {} as harness contract", harness_id.name))?;
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

    // Create test chain (empty sandbox; harnesses call vm.fork to opt into remote state).
    // RPC retries/timeout/etc. use ForkDBConfig defaults; override via vm.fork(..., ForkConfig).
    let fork_defaults = ForkDBConfig::new("").cache_dir(ripfuzz_dir(&project_path).join("cache"));
    let chain_config = ChainConfig::new(&project_path)
        .with_compiled_contracts(compiled_contracts)
        .with_fork_defaults(fork_defaults)
        .coverage(true);
    let mut chain = Chain::new(chain_config)?;

    // Deploy harness contract
    let contract_name = &harness_contract.artifact_id.name;
    console.begin(format!("deploying {contract_name}..."))?;
    let mut deploy_opts = DeployInput::new(&harness_contract.initcode)
        .caller(args.deployer_address)
        .value(args.deploy_value);
    let libraries = harness_contract.libraries.clone();
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
        let trace_dir = ripfuzz_dir(&project_path)
            .join("campaigns")
            .join(&campaign_id);
        fs::create_dir_all(&trace_dir)?;
        let trace_file = trace_dir.join("trace.log");
        let trace = deployment.trace.display_with(&ctx);
        fs::write(&trace_file, format!("{trace}"))?;
        console.end_fail(format!("failed to deploy {contract_name}"))?;
        console.print_line(format!("    trace: {}", trace_file.display()))?;
        return Err(anyhow::anyhow!("harness contract deployment failed"));
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
    let mut setup_coverage = None;
    if let Some(ref setup) = harness_contract.setup_function {
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
            let trace_dir = ripfuzz_dir(&project_path)
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
        setup_coverage = Some(setup_output.coverage);
        console.end()?;
    }

    // Extract literals from build artifacts so the fuzzer can seed random value
    // generation with concrete values found across the entire project.
    let literals = ExtractedLiterals::from_artifacts(&build_artifacts);
    let base_corpus_dir = args
        .corpus_dir
        .clone()
        .unwrap_or_else(|| ripfuzz_dir(&project_path).join("corpus"));
    let corpus_dir = SharedCorpus::dir_for(&base_corpus_dir, &harness_contract.artifact_id);
    let corpus_config = CorpusConfig::new(corpus_dir)
        .handler_functions(harness_contract.handler_functions.clone())
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
    shared_coverage.merge(&deployment.coverage);
    if let Some(coverage) = setup_coverage {
        shared_coverage.merge(&coverage);
    }
    let replay_count = corpus_stats.valid_count;

    if replay_count > 0 {
        console.begin(format!("replaying {replay_count} corpus items ..."))?;
        let replay_invariants = if args.max_mode {
            Vec::new()
        } else {
            harness_contract.invariant_functions.clone()
        };
        if let Err(e) = CorpusReplayer::new(shared_coverage.clone())
            .shared_corpus(corpus.clone())
            .chain(chain.clone())
            .deployed_address(deployed_address)
            .invariant_functions(replay_invariants)
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

    if args.max_mode {
        if harness_contract.max_functions.is_empty() {
            console.print_fail("max mode requires at least one `max_*` function in the harness")?;
            console.print_line(
                "harness contract must declare at least one `max_*` function in --max-mode",
            )?;
            return Err(anyhow::anyhow!(
                "harness contract must declare at least one `max_*` function in --max-mode"
            ));
        }
        return run_max_campaign(MaxCampaign {
            args: &args,
            console: &mut console,
            project: &project,
            chain: &chain,
            harness_contract: &harness_contract,
            deployed_address,
            campaign_id: &campaign_id,
            campaign_seed,
            build_artifacts: &build_artifacts,
            external_artifacts: &external_artifacts,
            corpus,
            shared_coverage,
            literals,
        });
    }

    // Initialize shared metrics across all fuzzer threads.
    let all_function_signatures: Vec<String> = harness_contract
        .handler_functions
        .iter()
        .chain(harness_contract.invariant_functions.iter())
        .map(|f| f.signature())
        .collect();
    let shared_metrics = SharedMetrics::new(all_function_signatures.clone());
    let shared_failed_assertions = SharedFailedAssertions::new(args.max_failures);

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
        .shared_failed_assertions(shared_failed_assertions.clone())
        .shutdown_signal(shutdown_signal.clone())
        .invariant_functions(harness_contract.invariant_functions.clone())
        .caller(args.deployer_address)
        .gas_limit(args.gas_limit)
        .timeout(timeout)
        .fail_on_revert(args.fail_on_revert);

    let mut handles = Vec::with_capacity(fuzzers);
    for fuzzer_id in 0..fuzzers {
        let local_max_runs = if fuzzer_id < remainder {
            base_runs + 1
        } else {
            base_runs
        };
        let seed = campaign_seed.wrapping_add(fuzzer_id as u64);
        // checkrs: allow(clone_in_loops)
        let mut config = initial_config.clone();
        config.max_runs = local_max_runs;
        config.seed = seed;

        let fuzzer = Fuzzer::new(config);
        let handle = std::thread::spawn(move || fuzzer.run());
        handles.push((fuzzer_id, handle));
    }

    let contract_name = &harness_contract.artifact_id.name;
    console.print(format!("fuzzing {contract_name} with {fuzzers} threads"))?;

    // Print a compact progress line every 3 seconds, then a full stats
    // summary after all fuzzer threads finish.
    let stats_ctx = formatter::CampaignStats::new(
        &shared_coverage,
        &corpus,
        &harness_contract.handler_functions,
        &harness_contract.invariant_functions,
        &[],
    );

    while handles.iter().any(|(_, h)| !h.is_finished()) {
        if let Some(snapshot) = shared_metrics.try_snapshot() {
            console.print_progress(stats_ctx.progress(&snapshot))?;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    for (fuzzer_id, handle) in handles {
        match handle.join() {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                tracing::error!(fuzzer_id, %e, "fuzzer failed");
            }
            Err(e) => {
                tracing::error!(fuzzer_id, ?e, "fuzzer panicked");
            }
        }
    }

    let failed_assertions = shared_failed_assertions.items();
    if failed_assertions.is_empty() {
        console.print_success(format!("fuzzed {contract_name} with {fuzzers} threads"))?;
        let function_metrics = shared_metrics.function_metrics();
        let stats = stats_ctx.format(&shared_metrics.aggregate(), &function_metrics);
        console.print_line(stats)?;
        console.new_line()?;

        let mut artifacts: Vec<Artifact> = build_artifacts.values().cloned().collect();
        artifacts.extend(external_artifacts.clone());
        let n = artifacts.len();
        console.begin(format!(
            "generating coverage reports for {n} build artifacts ..."
        ))?;
        match write_coverage_report(
            &project,
            &campaign_id,
            &shared_coverage,
            &harness_contract,
            artifacts,
        ) {
            Ok(files) => {
                console.update(format!(
                    "generated coverage reports for {n} build artifacts"
                ))?;
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
        console.print("ripfuzz out. see ya")?;

        return Ok(());
    }

    let shrink_threads = args.shrink_threads.unwrap_or(args.threads);
    let shrink_timeout = args.shrink_timeout_secs.map(std::time::Duration::from_secs);

    let invariant_calls: Vec<Call> = harness_contract
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

    // Include both handler and invariant functions so the shrinker can
    // generate replacement calls for any position in the sequence.
    let all_functions: Vec<alloy_json_abi::Function> = harness_contract
        .handler_functions
        .iter()
        .chain(harness_contract.invariant_functions.iter())
        .cloned()
        .collect();

    let failed_corpus_config = CorpusConfig::new(PathBuf::new())
        .handler_functions(all_functions)
        .max_calls(args.max_calls)
        .literals(literals);

    console.print_success(format!("fuzzed {contract_name} with {fuzzers} threads"))?;
    let function_metrics = shared_metrics.function_metrics();
    let stats = stats_ctx.format(&shared_metrics.aggregate(), &function_metrics);
    console.print_line(stats)?;
    console.new_line()?;
    let assertion_word = if failed_assertions.len() == 1 {
        "assertion"
    } else {
        "assertions"
    };
    console.print_fail(format!(
        "found {} distinct failed {assertion_word}",
        failed_assertions.len()
    ))?;

    let runs_per_assertion = (args.shrink_runs / failed_assertions.len() as u64).max(1);
    let mut shrunk_assertions = Vec::with_capacity(failed_assertions.len());

    for (assertion_index, assertion) in failed_assertions.iter().enumerate() {
        let assertion_number = assertion_index + 1;
        let initial_calls = assertion.item.calls.len();

        // Combine the failing item with invariants so the shrinker operates
        // on a single corpus item and never appends invariants.
        // checkrs: allow(clone_in_loops)
        let mut combined_calls = assertion.item.calls.clone();
        // checkrs: allow(clone_in_loops)
        combined_calls.extend(invariant_calls.clone());
        let combined_item = Item::from(combined_calls);
        let shared_failed_item =
            // checkrs: allow(clone_in_loops)
            SharedFailedCorpusItem::new(combined_item, failed_corpus_config.clone());

        let shrinker_shutdown = Arc::new(AtomicBool::new(false));
        // checkrs: allow(clone_in_loops)
        let shrinker_metrics = SharedMetrics::new(all_function_signatures.clone());

        let shrinkers_u64 = shrink_threads as u64;
        let base_shrink_runs = runs_per_assertion / shrinkers_u64;
        let shrink_remainder = (runs_per_assertion % shrinkers_u64) as usize;

        let mut shrinker_handles = Vec::with_capacity(shrink_threads);
        for shrinker_id in 0..shrink_threads {
            let local_max_runs = if shrinker_id < shrink_remainder {
                base_shrink_runs + 1
            } else {
                base_shrink_runs
            };
            let seed = campaign_seed
                .wrapping_add(shrinker_id as u64)
                .wrapping_add(1000 + assertion_index as u64 * 1000);
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

        console.print(format!(
            "shrinking assertion {assertion_number}/{} from {} calls with {} threads",
            failed_assertions.len(),
            formatter::num(initial_calls as u64),
            formatter::num(shrink_threads as u64)
        ))?;
        while shrinker_handles.iter().any(|h| !h.is_finished()) {
            if let Some(snapshot) = shrinker_metrics.try_snapshot() {
                let current_calls = shared_failed_item.item().calls.len();
                console.print_progress(formatter::shrinker_progress(
                    &snapshot,
                    initial_calls,
                    current_calls,
                ))?;
            }
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

        let shrunk_item = shared_failed_item.item();
        let shrunk_calls = shrunk_item.calls.len();
        console.print_success(format!(
            "shrank assertion {assertion_number}/{} from {} to {} calls with {} threads",
            failed_assertions.len(),
            formatter::num(initial_calls as u64),
            formatter::num(shrunk_calls as u64),
            formatter::num(shrink_threads as u64)
        ))?;
        let snapshot = shrinker_metrics.aggregate();
        console.print_line(formatter::shrinker_summary(
            &snapshot,
            initial_calls,
            shrunk_calls,
        ))?;
        console.new_line()?;
        shrunk_assertions.push((assertion_number, shrunk_item));
    }

    // Re-run each shrunk item with the chain tracer enabled.
    for (assertion_number, shrunk_item) in &shrunk_assertions {
        // checkrs: allow(clone_in_loops)
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
            console.begin(format!("writing trace {assertion_number} ..."))?;
            let trace_name = if failed_assertions.len() == 1 {
                "trace.log".to_owned()
            } else {
                format!("trace-{assertion_number}.log")
            };
            let trace_file = match write_trace_to_file(
                &trace,
                &project,
                &campaign_id,
                deployed_address,
                contract_name,
                &chain,
                &trace_name,
            ) {
                Ok(f) => f,
                Err(e) => {
                    console.end_fail("writing trace file failed")?;
                    console.print_line(format!("{e:#}"))?;
                    return Err(e);
                }
            };
            console.update(format!(
                "trace {assertion_number}: {}",
                trace_file.display()
            ))?;
            console.end()?;
        }
    }

    let mut artifacts: Vec<Artifact> = build_artifacts.values().cloned().collect();
    artifacts.extend(external_artifacts.clone());
    let n = artifacts.len();
    console.begin(format!(
        "generating coverage reports for {n} build artifacts ..."
    ))?;
    match write_coverage_report(
        &project,
        &campaign_id,
        &shared_coverage,
        &harness_contract,
        artifacts,
    ) {
        Ok(files) => {
            console.update(format!(
                "generated coverage reports for {n} build artifacts"
            ))?;
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

    console.print("ripfuzz out. see ya")?;
    Ok(())
}

/// Shared context for a max-mode campaign.
struct MaxCampaign<'a, W> {
    args: &'a Args,
    console: &'a mut Console<W>,
    project: &'a Project,
    chain: &'a Chain,
    harness_contract: &'a Contract,
    deployed_address: Address,
    campaign_id: &'a str,
    campaign_seed: u64,
    build_artifacts: &'a HashMap<ArtifactId, Artifact>,
    external_artifacts: &'a [Artifact],
    corpus: SharedCorpus,
    shared_coverage: SharedCoverage,
    literals: ExtractedLiterals,
}

fn run_max_campaign<W: std::io::Write>(campaign: MaxCampaign<'_, W>) -> Result<()> {
    let MaxCampaign {
        args,
        console,
        project,
        chain,
        harness_contract,
        deployed_address,
        campaign_id,
        campaign_seed,
        build_artifacts,
        external_artifacts,
        corpus,
        shared_coverage,
        literals,
    } = campaign;
    let contract_name = &harness_contract.artifact_id.name;

    ensure!(
        !harness_contract.max_functions.is_empty(),
        "harness contract must declare at least one `max_*` function when --max-mode is enabled"
    );

    let all_function_signatures: Vec<String> = harness_contract
        .handler_functions
        .iter()
        .chain(harness_contract.max_functions.iter())
        .map(|f| f.signature())
        .collect();
    let shared_metrics = SharedMetrics::new(all_function_signatures.clone());
    let shutdown_signal = Arc::new(AtomicBool::new(false));

    let objectives: Vec<MaxObjective> = harness_contract
        .max_functions
        .iter()
        .cloned()
        .map(MaxObjective::new)
        .collect();
    let fuzzer_corpus = MaxFuzzerCorpus::new(corpus.clone(), objectives.len());

    let fuzzers = args.threads;
    let timeout = args.timeout_secs.map(std::time::Duration::from_secs);
    let fuzzers_u64 = fuzzers as u64;
    let base_runs = args.max_runs / fuzzers_u64;
    let remainder = (args.max_runs % fuzzers_u64) as usize;

    let initial_config = MaxFuzzerConfig::new()
        .chain(chain.clone())
        .target_address(deployed_address)
        .shared_corpus(fuzzer_corpus.clone())
        .shared_coverage(shared_coverage.clone())
        .shared_metrics(shared_metrics.clone())
        .shutdown_signal(shutdown_signal.clone())
        .caller(args.deployer_address)
        .objectives(objectives.clone())
        .gas_limit(args.gas_limit)
        .timeout(timeout);

    let mut handles = Vec::with_capacity(fuzzers);
    for fuzzer_id in 0..fuzzers {
        let local_max_runs = if fuzzer_id < remainder {
            base_runs + 1
        } else {
            base_runs
        };
        let seed = campaign_seed.wrapping_add(fuzzer_id as u64);
        // checkrs: allow(clone_in_loops)
        let mut config = initial_config.clone();
        config.max_runs = local_max_runs;
        config.seed = seed;

        let fuzzer = MaxFuzzer::new(config);
        let handle = std::thread::spawn(move || fuzzer.run());
        handles.push((fuzzer_id, handle));
    }

    console.print(format!(
        "max fuzzing {contract_name} with {fuzzers} threads"
    ))?;

    let stats_ctx = formatter::CampaignStats::new(
        &shared_coverage,
        &corpus,
        &harness_contract.handler_functions,
        &[],
        &harness_contract.max_functions,
    );

    while handles.iter().any(|(_, h)| !h.is_finished()) {
        if let Some(snapshot) = shared_metrics.try_snapshot() {
            console.print_progress(stats_ctx.progress(&snapshot))?;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    for (fuzzer_id, handle) in handles {
        match handle.join() {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                tracing::error!(fuzzer_id, %e, "max fuzzer failed");
            }
            Err(e) => {
                tracing::error!(fuzzer_id, ?e, "max fuzzer panicked");
            }
        }
    }

    console.print_success(format!("fuzzed {contract_name} with {fuzzers} threads"))?;
    let function_metrics = shared_metrics.function_metrics();
    let stats = stats_ctx.format(&shared_metrics.aggregate(), &function_metrics);
    console.print_line(stats)?;
    console.new_line()?;

    let best_items = fuzzer_corpus.best_items();
    let shrink_config = CorpusConfig::new(PathBuf::new())
        .handler_functions(harness_contract.handler_functions.clone())
        .max_calls(args.max_calls)
        .literals(literals.clone());
    let shrink_threads = args.shrink_threads.unwrap_or(args.threads);
    let shrink_timeout = args.shrink_timeout_secs.map(std::time::Duration::from_secs);

    let mut results = Vec::new();
    for (index, best) in best_items.into_iter().enumerate() {
        let Some(best) = best else {
            continue;
        };
        // checkrs: allow(clone_in_loops)
        let objective = objectives[index].clone();
        // checkrs: allow(clone_in_loops)
        let shrink_config = shrink_config.clone();
        // checkrs: allow(clone_in_loops)
        let corpus = corpus.clone();
        let shrink_corpus = MaxShrinkerCorpus::new(best.item, best.value, shrink_config, corpus);

        let runs_per_result = args.shrink_runs.max(1);
        let shrinkers_u64 = shrink_threads as u64;
        let base_shrink_runs = (runs_per_result / shrinkers_u64).max(1);
        let shrink_remainder = (runs_per_result % shrinkers_u64) as usize;
        let shrinker_shutdown = Arc::new(AtomicBool::new(false));
        // checkrs: allow(clone_in_loops)
        let shrinker_metrics = SharedMetrics::new(all_function_signatures.clone());

        let mut shrinker_handles = Vec::with_capacity(shrink_threads);
        for shrinker_id in 0..shrink_threads {
            let local_max_runs = if shrinker_id < shrink_remainder {
                base_shrink_runs + 1
            } else {
                base_shrink_runs
            };
            let seed = campaign_seed
                .wrapping_add(shrinker_id as u64)
                .wrapping_add(2000 + index as u64 * 1000);
            // checkrs: allow(clone_in_loops)
            let shrinker_chain = chain.clone();
            // checkrs: allow(clone_in_loops)
            let shrinker_corpus = shrink_corpus.clone();
            // checkrs: allow(clone_in_loops)
            let shrinker_shutdown = shrinker_shutdown.clone();
            // checkrs: allow(clone_in_loops)
            let shrinker_objective = objective.clone();
            let shrinker_config = MaxShrinkerConfig::new()
                .chain(shrinker_chain)
                .target_address(deployed_address)
                .shared_corpus(shrinker_corpus)
                .shutdown_signal(shrinker_shutdown)
                .objective(shrinker_objective)
                .max_runs(local_max_runs)
                .timeout(shrink_timeout)
                .seed(seed)
                // checkrs: allow(clone_in_loops)
                .shared_metrics(shrinker_metrics.clone())
                .gas_limit(args.gas_limit)
                .caller(args.deployer_address);
            let shrinker = MaxShrinker::new(shrinker_config);
            let handle = std::thread::spawn(move || shrinker.run());
            shrinker_handles.push(handle);
        }

        let initial_calls = shrink_corpus.item().item.calls.len();
        console.print(format!(
            "shrinking max {} from {} calls with {} threads",
            objective.function.name,
            formatter::num(initial_calls as u64),
            formatter::num(shrink_threads as u64)
        ))?;
        while shrinker_handles.iter().any(|h| !h.is_finished()) {
            if let Some(snapshot) = shrinker_metrics.try_snapshot() {
                let current_calls = shrink_corpus.item().item.calls.len();
                console.print_progress(formatter::shrinker_progress(
                    &snapshot,
                    initial_calls,
                    current_calls,
                ))?;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        for handle in shrinker_handles {
            match handle.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    tracing::error!(%e, "max shrinker failed");
                }
                Err(e) => {
                    tracing::error!(?e, "max shrinker panicked");
                }
            }
        }

        let shrunk = shrink_corpus.item();
        let shrunk_calls = shrunk.item.calls.len();
        console.print_success(format!(
            "shrank max {} from {} to {} calls with {} threads",
            objective.function.name,
            formatter::num(initial_calls as u64),
            formatter::num(shrunk_calls as u64),
            formatter::num(shrink_threads as u64)
        ))?;
        console.new_line()?;
        results.push(MaxResult {
            objective,
            value: shrunk.value,
            item: shrunk.item,
        });
    }

    if results.is_empty() {
        console.print_fail("no max value improved above 0")?;
    } else {
        for result in &results {
            console.print_line(format!(
                "    max {} = {}",
                result.objective.function.name, result.value
            ))?;
            console.print_line(result.format_call_sequence())?;
            console.new_line()?;
        }
    }

    // Re-run each shrunk item with the chain tracer enabled.
    for (index, result) in results.iter().enumerate() {
        // checkrs: allow(clone_in_loops)
        let mut trace_chain = chain.clone();
        trace_chain.set_trace(true);

        let mut transactions: Vec<Transaction> = result
            .item
            .calls
            .iter()
            .map(|call| call.into_transaction(deployed_address))
            .collect();
        transactions.push(result.objective.transaction(
            deployed_address,
            args.deployer_address,
            args.gas_limit,
        ));

        let exec = trace_chain.exec(&transactions)?;

        if let Some(trace) = exec.trace {
            console.begin(format!("writing max trace {} ...", index + 1))?;
            let trace_file = write_trace_to_file(
                &trace,
                project,
                campaign_id,
                deployed_address,
                contract_name,
                chain,
                &format!("trace-max-{}.log", index + 1),
            )?;
            console.update(format!("max trace {}: {}", index + 1, trace_file.display()))?;
            console.end()?;
        }
    }

    let mut artifacts: Vec<Artifact> = build_artifacts.values().cloned().collect();
    artifacts.extend(external_artifacts.iter().cloned());
    let n = artifacts.len();
    console.begin(format!(
        "generating coverage reports for {n} build artifacts ..."
    ))?;
    match write_coverage_report(
        project,
        campaign_id,
        &shared_coverage,
        harness_contract,
        artifacts,
    ) {
        Ok(files) => {
            console.update(format!(
                "generated coverage reports for {n} build artifacts"
            ))?;
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

    console.print("ripfuzz out. see ya")?;
    Ok(())
}

fn write_coverage_report(
    project: &Project,
    campaign_id: &str,
    shared_coverage: &SharedCoverage,
    _harness_contract: &Contract,
    artifacts: Vec<Artifact>,
) -> Result<Vec<(PathBuf, f64)>> {
    let reporter = CoverageReporter::new()
        .build_artifacts(artifacts)
        .shared_coverage(shared_coverage.clone())
        .base_project_path(&project.path);

    let report = reporter.build();

    let coverage_dir = ripfuzz_dir(&project.path)
        .join("campaigns")
        .join(campaign_id)
        .join("coverage");
    fs::create_dir_all(&coverage_dir)?;

    let lcov_file = coverage_dir.join("lcov.info");
    let lcov_content = format!("{report}");
    fs::write(&lcov_file, &lcov_content)?;

    let relative_path = lcov_file
        .strip_prefix(&project.path)
        .unwrap_or(&lcov_file)
        .to_path_buf();
    let pct = report.coverage();

    Ok(vec![(relative_path, pct)])
}

fn write_trace_to_file(
    trace: &Trace,
    project: &Project,
    campaign_id: &str,
    deployed_address: Address,
    contract_name: &str,
    chain: &Chain,
    trace_file_name: &str,
) -> Result<PathBuf> {
    let mut ctx = TraceContext::from_project(project)?.with_label(deployed_address, contract_name);
    for (addr, label) in chain.labels() {
        ctx = ctx.with_label(*addr, label);
    }
    let trace_dir = ripfuzz_dir(&project.path)
        .join("campaigns")
        .join(campaign_id);
    fs::create_dir_all(&trace_dir)?;
    let trace_file = trace_dir.join(trace_file_name);
    let trace_str = trace.display_with(&ctx);
    fs::write(&trace_file, format!("{trace_str}"))?;
    Ok(trace_file)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use revm::primitives::U256;

    use crate::evm::DEFAULT_DEPLOYER;

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
            harness: "src/L1SimpleKnob.sol:SimpleKnob".to_owned(),
            project_path: Some(PathBuf::from("fixtures/challenges")),
            deploy_value: U256::ZERO,
            deployer_address: DEFAULT_DEPLOYER,
            threads: 1,
            max_runs: 10000,
            max_failures: 1,
            timeout_secs: None,
            gas_limit: 12_500_000,
            max_calls: 32,
            seed: Some(0),
            corpus_dir: Some(corpus_dir),
            log_level: tracing::Level::INFO,
            disable_log: true,
            ffi: false,
            force: false,
            fail_on_revert: false,
            max_mode: false,
            external_projects: Vec::new(),
            shrink_runs: 1,
            shrink_timeout_secs: None,
            shrink_threads: None,
        }
    }

    /// Present `.env` must load values into the process environment.
    #[test]
    fn load_dotenv_loads_values() {
        let dir = tempfile::tempdir().unwrap();
        let key = format!("RIPFUZZ_DOTENV_LOAD_{}", std::process::id());
        fs::write(dir.path().join(".env"), format!("{key}=from-dotenv\n")).unwrap();

        let loaded = load_dotenv(dir.path()).expect("load must succeed");
        assert_eq!(
            loaded.as_deref(),
            Some(dir.path().join(".env").as_path()),
            "must return the loaded .env path"
        );
        assert_eq!(
            env::var(&key).expect("env var must be set"),
            "from-dotenv",
            "must load values from .env"
        );
    }

    /// Existing process environment variables must take precedence over `.env`.
    #[test]
    fn load_dotenv_preserves_existing_env() {
        let original = env::var("PATH").expect("PATH must be set");
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".env"), "PATH=should-not-override\n").unwrap();

        load_dotenv(dir.path()).expect("load must succeed");
        assert_eq!(
            env::var("PATH").expect("PATH must still be set"),
            original,
            "existing env vars must not be overridden by .env"
        );
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

    /// Max mode must find a positive max value and persist the improving
    /// sequence to the corpus.
    #[test]
    fn max_mode_finds_positive_value() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus_dir = tmp.path().join("corpus");

        let mut args = make_args(corpus_dir.clone());
        args.harness = "src/MaxBasic.sol:MaxBasic".to_owned();
        args.project_path = Some(PathBuf::from("fixtures/max-mode"));
        args.max_mode = true;
        args.max_runs = 1000;
        args.max_calls = 4;
        args.shrink_runs = 500;

        run(args).expect("max mode run should succeed");
        assert!(
            count_corpus_files(&corpus_dir) > 0,
            "max mode should persist improving sequences to the corpus"
        );
    }

    /// Max mode must ignore invariant functions entirely.
    #[test]
    fn max_mode_ignores_invariants() {
        let tmp = tempfile::tempdir().unwrap();

        let mut args = make_args(tmp.path().join("corpus"));
        args.harness = "src/MaxMixed.sol:MaxMixed".to_owned();
        args.project_path = Some(PathBuf::from("fixtures/max-mode"));
        args.max_mode = true;
        args.max_runs = 200;
        args.max_calls = 4;
        args.shrink_runs = 100;

        run(args).expect("max mode must succeed even when invariants would fail");
    }

    /// Invariant mode must ignore max functions and still report assertion
    /// failures from the same harness.
    #[test]
    fn invariant_mode_reports_mixed_harness_failure() {
        let tmp = tempfile::tempdir().unwrap();

        let mut args = make_args(tmp.path().join("corpus"));
        args.harness = "src/MaxMixed.sol:MaxMixed".to_owned();
        args.project_path = Some(PathBuf::from("fixtures/max-mode"));
        args.max_mode = false;
        args.max_runs = 200;
        args.max_calls = 4;
        args.shrink_runs = 100;

        run(args).expect("invariant mode run should succeed");
    }

    /// Max mode without any `max_*` function must fail with a clear error.
    #[test]
    fn max_mode_requires_max_functions() {
        let tmp = tempfile::tempdir().unwrap();

        let mut args = make_args(tmp.path().join("corpus"));
        args.max_mode = true;
        args.max_runs = 100;

        let err = run(args).expect_err("max mode without max functions must fail");
        assert!(
            err.to_string().contains("at least one `max_*` function"),
            "unexpected error: {err}"
        );
    }
}
