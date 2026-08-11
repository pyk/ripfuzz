//! Shared session state and setup for fuzzing campaigns.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use alloy_primitives::Address;
use anyhow::{Context, Result, ensure};
use revm::primitives::Bytes;
use tracing::{debug, error, info, warn};

use crate::campaigns::CampaignKind;
use crate::commands::run::Args;
use crate::corpus::{CorpusConfig, CorpusReplayer, ExtractedLiterals, SharedCorpus};
use crate::evm::{
    Chain, ChainConfig, Contract, CoverageReporter, DeployInput, ForkDBConfig, SetupInput,
    SharedCoverage, Trace, TraceContext, Transaction,
};
use crate::formatter;
use crate::foundry::{Artifact, ArtifactId, BuildOptions, Project};
use crate::logger;

/// Returns the ripfuzz data directory for the given project path.
fn ripfuzz_dir(project_path: impl AsRef<Path>) -> PathBuf {
    project_path.as_ref().join(".ripfuzz")
}

/// Campaign output directory: log file, traces, and coverage reports.
fn campaign_dir(project_path: impl AsRef<Path>, campaign_id: &str) -> PathBuf {
    ripfuzz_dir(project_path)
        .join("campaigns")
        .join(campaign_id)
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

/// Validate a harness that enters max mode.
///
/// Max mode is entered automatically when the harness declares at least one
/// `max_*` function. It supports exactly one max function and cannot be
/// combined with `invariant_*` functions.
fn validate_harness_mode(harness: &Contract) -> Result<()> {
    if harness.max_functions.is_empty() {
        return Ok(());
    }

    let max_names = harness
        .max_functions
        .iter()
        .map(|f| format!("`{}`", f.name))
        .collect::<Vec<String>>()
        .join(", ");
    ensure!(
        harness.max_functions.len() == 1,
        "max mode supports exactly one `max_*` function, but harness `{}` declares {}: {}",
        harness.artifact_id.name,
        harness.max_functions.len(),
        max_names
    );

    let invariant_names = harness
        .invariant_functions
        .iter()
        .map(|f| format!("`{}`", f.name))
        .collect::<Vec<String>>()
        .join(", ");
    ensure!(
        harness.invariant_functions.is_empty(),
        "harness `{}` enters max mode via {} but also declares `invariant_*` function(s): {}; max mode does not support invariants",
        harness.artifact_id.name,
        max_names,
        invariant_names
    );

    Ok(())
}

/// A re-run transaction sequence: the trace file holding the full trace and
/// the compact trace (call context and storage changes omitted) for stderr.
pub struct TraceReport {
    pub file: PathBuf,
    pub compact: String,
}

/// Shared state for one campaign, built once and consumed by a campaign type.
pub struct CampaignSession {
    pub args: Args,
    pub project: Project,
    pub chain: Chain,
    pub harness_contract: Contract,
    pub deployed_address: Address,
    pub campaign_id: String,
    pub campaign_seed: u64,
    pub build_artifacts: HashMap<ArtifactId, Artifact>,
    pub external_artifacts: Vec<Artifact>,
    pub corpus: SharedCorpus,
    pub shared_coverage: SharedCoverage,
    pub literals: ExtractedLiterals,
    pub kind: CampaignKind,
    /// Campaign log file path; `None` when logging is disabled.
    pub log_file: Option<PathBuf>,
}

impl CampaignSession {
    /// Resolve the project, compile, deploy, and prepare the corpus.
    pub fn new(args: Args) -> Result<Self> {
        // Resolve the campaign seed early so it can be logged before any work.
        let campaign_seed = match args.seed {
            Some(seed) => {
                info!(
                    "Starting ripfuzz v{} (seed: {seed}, user-provided)",
                    env!("CARGO_PKG_VERSION")
                );
                seed
            }
            None => {
                let seed = fastrand::Rng::new().u64(1..=100_000);
                info!(
                    "Starting ripfuzz v{} (seed: {seed})",
                    env!("CARGO_PKG_VERSION")
                );
                seed
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

        let log_file = ripfuzz_dir(&project_path)
            .join("campaigns")
            .join(&campaign_id)
            .join("fuzz.log");
        logger::init(args.disable_log, &log_file, args.log_level)?;

        debug!(?project_path, "Resolved project path");
        if let Some(path) = &dotenv_path {
            debug!(?path, "Loaded environment from .env");
        }

        // Build project
        info!("Building foundry project");
        let project = Project::new(&project_path);
        let build_opts = BuildOptions::new().force(args.force);
        project.build(build_opts)?;

        // Load build artifacts
        debug!("Loading build artifacts");
        let build_artifacts = project.load_artifacts()?;
        info!(
            "Loaded {} build artifacts",
            formatter::num(build_artifacts.len() as u64)
        );

        // Load external project artifacts for coverage and trace resolution.
        let mut external_artifacts = Vec::new();
        for ext_path in &args.external_projects {
            let ext_project = Project::new(ext_path);
            match ext_project.load_artifacts() {
                Ok(artifacts) => {
                    info!(
                        "Loaded {} artifacts from external project {}",
                        formatter::num(artifacts.len() as u64),
                        ext_path.display()
                    );
                    for (_, mut artifact) in artifacts {
                        artifact.set_project_path(ext_path);
                        external_artifacts.push(artifact);
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to load artifacts from {}: {e:#}",
                        ext_path.display()
                    );
                }
            }
        }

        // Resolve the harness (bare name or full artifact id) then load it.
        debug!("Loading harness contract {}", args.harness);
        let harness_id = ArtifactId::resolve(&args.harness, &build_artifacts)?;
        let harness_contract = Contract::try_get(&build_artifacts, &harness_id)?;
        info!("Loaded harness contract {}", harness_id.name);

        // Max mode is entered automatically whenever the harness declares at
        // least one `max_*` function. Invariant mode is the default otherwise.
        let max_mode = !harness_contract.max_functions.is_empty();
        if max_mode && let Err(e) = validate_harness_mode(&harness_contract) {
            error!("Harness contract is not valid for max mode");
            error!("{e:#}");
            return Err(e);
        }
        let kind = if max_mode {
            CampaignKind::Maxxing
        } else {
            CampaignKind::Invariant
        };

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

        // Create test chain (empty sandbox; harnesses call vm.fork to opt into
        // remote state). RPC retries/timeout/etc. use ForkDBConfig defaults;
        // override via vm.fork(..., ForkConfig).
        let fork_defaults =
            ForkDBConfig::new("").cache_dir(ripfuzz_dir(&project_path).join("cache"));
        let chain_config = ChainConfig::new(&project_path)
            .with_compiled_contracts(compiled_contracts)
            .with_fork_defaults(fork_defaults)
            .coverage(true);
        let mut chain = Chain::new(chain_config)?;

        // Deploy harness contract
        let contract_name = &harness_contract.artifact_id.name;
        let mut deploy_opts = DeployInput::new(&harness_contract.initcode)
            .caller(args.deployer_address)
            .value(args.deploy_value);
        let libraries = harness_contract.libraries.clone();
        for lib in libraries {
            deploy_opts = deploy_opts.add_library(lib);
        }

        debug!("Deploying {contract_name}");
        let deployment = chain.deploy(deploy_opts)?;
        if !deployment.result.success {
            let mut ctx = TraceContext::from_artifacts(build_artifacts.clone());
            if let Some(addr) = deployment.trace.roots.first().and_then(|r| r.address) {
                ctx = ctx.with_label(addr, contract_name);
            }
            for (addr, label) in chain.labels() {
                ctx = ctx.with_label(*addr, label);
            }
            let trace_dir = campaign_dir(&project_path, &campaign_id);
            fs::create_dir_all(&trace_dir)?;
            let trace_file = trace_dir.join("trace.log");
            let trace = deployment.trace.display_with(&ctx);
            fs::write(&trace_file, format!("{trace}"))?;
            error!("Failed to deploy {contract_name}");
            error!("    trace: {}", trace_file.display());
            return Err(anyhow::anyhow!("harness contract deployment failed"));
        }
        let deployed_address = deployment
            .address
            .context("deployment succeeded but created_address is missing")?;

        let contract_size = deployment
            .result
            .output
            .as_ref()
            .map(|b| b.len())
            .unwrap_or(0);
        info!(
            deployer = %args.deployer_address,
            msg_value = %formatter::eth(args.deploy_value),
            address = %deployed_address,
            contract_size = %formatter::kb(contract_size),
            "Deployed {contract_name}",
        );

        // Run setup if present
        let mut setup_coverage = None;
        if let Some(setup) = &harness_contract.setup_function {
            debug!("Calling setup");
            let setup_output = match chain.setup(
                SetupInput::new(deployed_address)
                    .calldata(Bytes::from(setup.selector().as_slice().to_vec()))
                    .caller(args.deployer_address),
            ) {
                Ok(output) => output,
                Err(e) => {
                    error!("Calling setup failed: {e:#}");
                    return Err(e);
                }
            };
            if !setup_output.result.success {
                let mut ctx = TraceContext::from_artifacts(build_artifacts.clone())
                    .with_label(deployed_address, contract_name);
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
                error!("Failed to call setup");
                error!("    trace: {}", trace_file.display());
                return Err(anyhow::anyhow!("setup failed"));
            }
            setup_coverage = Some(setup_output.coverage);
            info!("Called setup");
        }

        // Extract literals from build artifacts so the fuzzer can seed random
        // value generation with concrete values found across the project.
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
            debug!("Loading corpus items");
            info!(
                on_disk = %formatter::num(corpus_stats.total_count as u64),
                valid = %formatter::num(corpus_stats.valid_count as u64),
                invalid = %formatter::num(
                    (corpus_stats.parse_failed_count + corpus_stats.invalid_call_count) as u64
                ),
                "Loaded {} corpus items",
                formatter::num(corpus_stats.valid_count as u64),
            );
        }

        // Initialize shared coverage and sync with corpus.
        let shared_coverage = SharedCoverage::new();
        shared_coverage.merge(&deployment.coverage);
        if let Some(coverage) = setup_coverage {
            shared_coverage.merge(&coverage);
        }
        let replay_count = corpus_stats.valid_count;

        if replay_count > 0 {
            debug!("Replaying {replay_count} corpus items");
            let replay_invariants = if max_mode {
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
                error!("Replaying corpus items failed: {e:#}");
                return Err(e);
            }
            info!(
                contracts = %formatter::num(shared_coverage.contract_count() as u64),
                edges = %formatter::num(shared_coverage.edge_count() as u64),
                depths = %formatter::num(shared_coverage.depth_count() as u64),
                reverts = %formatter::num(shared_coverage.revert_count() as u64),
                jumps = %formatter::num(shared_coverage.jump_count() as u64),
                "Replayed {replay_count} corpus items",
            );
        }

        let session_log_file = (!args.disable_log).then(|| log_file.clone());

        Ok(Self {
            args,
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
            kind,
            log_file: session_log_file,
        })
    }

    /// Name of the harness contract.
    pub fn contract_name(&self) -> &str {
        &self.harness_contract.artifact_id.name
    }

    /// Write a trace for the current campaign and return its path.
    pub fn write_trace(&self, trace: &Trace, file_name: &str) -> Result<PathBuf> {
        let ctx = self.trace_context();
        let trace_dir = campaign_dir(&self.project.path, &self.campaign_id);
        fs::create_dir_all(&trace_dir)?;
        let trace_file = trace_dir.join(file_name);
        let trace_str = trace.display_with(&ctx);
        fs::write(&trace_file, format!("{trace_str}"))?;
        Ok(trace_file)
    }

    /// Re-run `transactions` with tracing enabled, write the full trace to
    /// `file_name` in the campaign directory, and return the trace file path
    /// together with a compact rendering (call context and storage changes
    /// omitted) for stderr output.
    pub fn trace_sequence_to_file(
        &self,
        transactions: &[Transaction],
        file_name: &str,
    ) -> Result<TraceReport> {
        let mut trace_chain = self.chain.clone();
        trace_chain.set_trace(true);
        let exec = trace_chain.exec(transactions)?;
        let trace = exec.trace.context("trace expected after re-run")?;
        let ctx = self.trace_context();
        let full = format!("{}", trace.display_with(&ctx));
        let compact = format!("{}", trace.display_compact_with(&ctx));

        let trace_dir = campaign_dir(&self.project.path, &self.campaign_id);
        fs::create_dir_all(&trace_dir)?;
        let trace_file = trace_dir.join(file_name);
        fs::write(&trace_file, &full)?;
        Ok(TraceReport {
            file: trace_file,
            compact,
        })
    }

    /// Trace context for the current campaign: the already-loaded project
    /// artifacts plus chain labels.
    fn trace_context(&self) -> TraceContext {
        let mut ctx = TraceContext::from_artifacts(self.build_artifacts.clone())
            .with_label(self.deployed_address, self.contract_name());
        for (addr, label) in self.chain.labels() {
            ctx = ctx.with_label(*addr, label);
        }
        ctx
    }

    /// Generate coverage reports for the current campaign.
    pub fn write_coverage_report(&self) -> Result<()> {
        let mut artifacts: Vec<Artifact> = self.build_artifacts.values().cloned().collect();
        artifacts.extend(self.external_artifacts.iter().cloned());
        let n = artifacts.len();

        let reporter = CoverageReporter::new()
            .build_artifacts(artifacts)
            .shared_coverage(self.shared_coverage.clone())
            .base_project_path(&self.project.path);

        let report = reporter.build();

        let coverage_dir = ripfuzz_dir(&self.project.path)
            .join("campaigns")
            .join(&self.campaign_id)
            .join("coverage");
        fs::create_dir_all(&coverage_dir)?;

        let lcov_file = coverage_dir.join("lcov.info");
        let lcov_content = format!("{report}");
        fs::write(&lcov_file, &lcov_content)?;

        let relative_path = lcov_file
            .strip_prefix(&self.project.path)
            .unwrap_or(&lcov_file)
            .to_path_buf();
        let pct = report.coverage();

        info!("Generated coverage reports for {n} build artifacts");
        info!("    [{pct:.2}%] {}", relative_path.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;

    use super::*;

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
}
