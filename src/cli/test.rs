//! `test` CLI command implementation.

use std::fs;
use std::path::{Path, PathBuf, absolute};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use revm::primitives::Bytes;
use tracing::{error, info, warn};

use crate::compilers::solc::Solc;
use crate::config::Config;
use crate::evm::{
    Chain, ChainConfig, ForkDBConfig, SetupInput, SharedCoverage, Trace, TraceContext, Transaction,
};
use crate::harness::HarnessId;
use crate::tester::{
    BrokenInvariant, Corpus, Fuzzer, Replayer, SharedBrokenInvariants, Shrinker, TestHarness,
};

/// Find broken invariants.
#[derive(Debug, Parser)]
pub struct Args {
    /// Path to harness to run.
    #[arg(value_name = "HARNESS")]
    pub harness: HarnessId,

    /// Path to the ripfuzz config file.
    #[arg(long, default_value = "ripfuzz.toml", value_name = "PATH")]
    pub config: PathBuf,

    /// Project root directory.
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub root: PathBuf,

    /// Number of threads to utilize.
    #[arg(long, default_value_t = 1, value_name = "THREADS")]
    pub threads: usize,

    /// Maximum number of sequences to run across all threads.
    #[arg(long, default_value_t = 256, value_name = "RUNS")]
    pub max_runs: u64,

    /// Maximum number of handler calls per sequence.
    #[arg(long, default_value_t = 8, value_name = "COUNT")]
    pub max_calls: usize,

    /// Stop fuzzing after this many seconds.
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<u64>,

    /// Stop fuzzing after this many distinct broken invariants.
    #[arg(long, default_value_t = 256, value_name = "COUNT")]
    pub max_failures: usize,

    /// Directory to load and save the corpus.
    #[arg(long, default_value = ".ripfuzz/corpus", value_name = "PATH")]
    pub corpus_dir: PathBuf,

    /// Suppress terminal log output.
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Log verbosity level.
    #[arg(long, default_value = "info", value_name = "LEVEL")]
    pub log_level: tracing::Level,
}

/// Run the `test` command and return the shrunk broken invariants.
pub fn run(args: Args) -> Result<Vec<BrokenInvariant>> {
    // 1. Initialize the tracing subscriber.
    //
    //    Quiet mode writes to a null sink instead of skipping init, so a
    //    subscriber installed by an earlier caller (e.g. a test binary)
    //    cannot leak events into the terminal.
    let builder = tracing_subscriber::fmt()
        .with_max_level(args.log_level)
        .with_target(false);
    let _ = if args.quiet {
        builder.with_writer(std::io::sink).try_init()
    } else {
        builder.try_init()
    };

    // 2. Load configuration relative to the project root.
    let root = args.root;
    let config = Config::new().with_root(&root).load(&args.config)?;

    // 3. Compile the harness via Solc relative to the project root.
    let solc_output = Solc::new()
        .with_version(&config.solc.version)
        .with_root(&root)
        .with_target(&args.harness.path)
        .with_name(&args.harness.name)
        .with_out(&config.solc.out)
        .with_evm_version(config.solc.evm_version)
        .with_optimizer(config.solc.optimizer, config.solc.optimizer_runs)
        .with_via_ir(config.solc.via_ir)
        .with_remappings(config.solc.remappings.clone())
        .compile()?;

    // 4. Validate the compiled output against the test harness rules.
    let test_harness = TestHarness::try_from(&solc_output)?;

    // 5. Create the test chain the harness will be deployed to.
    //
    //    Forks share the on-disk RPC cache with other commands, and a
    //    conservative batch rate limit keeps default campaigns under
    //    public-provider quotas.
    let fork_defaults = ForkDBConfig::new("")
        .cache_dir(root.join(".ripfuzz").join("cache"))
        .rate_limit(Some(10));
    let chain_config = ChainConfig::new(&root)
        .with_fork_defaults(fork_defaults)
        .coverage(true);
    let mut chain = Chain::new(chain_config)?;

    // 6. Label the trace context from the compilation output and the chain.
    let mut trace_context = TraceContext::from(&solc_output);
    let labels = chain.labels().clone();
    for (address, label) in labels {
        trace_context = trace_context.with_label(address, label);
    }

    // 7. Create the shared coverage map so the deployment and setup calls
    //    below seed the baseline the fuzzers measure new edges against.
    let coverage = SharedCoverage::new();

    // 8. Deploy the harness contract.
    info!("deploying harness");
    let deployment = chain.deploy(&test_harness)?;
    if !deployment.result.success {
        let trace_file = dump_execution_trace(&root, &trace_context, &deployment.trace)?;
        error!("execution trace: {}", trace_file.display());
        bail!(
            "harness contract `{}` deployment failed",
            test_harness.id().name
        );
    }
    let address = deployment
        .address
        .context("deployment succeeded but created_address is missing")?;
    coverage.merge(&deployment.coverage);
    info!(address = %address, "harness deployed");

    // 9. Run the setup function if the harness defines one.
    // checkrs: allow(nested_if_let)
    if let Some(setup) = test_harness.setup() {
        let setup_input =
            SetupInput::new(address).calldata(Bytes::from(setup.selector().as_slice().to_vec()));
        let setup_output = chain.setup(setup_input)?;
        if !setup_output.result.success {
            let trace_file = dump_execution_trace(&root, &trace_context, &setup_output.trace)?;
            error!("execution trace: {}", trace_file.display());
            bail!("harness contract `{}` setup failed", test_harness.id().name);
        }
        coverage.merge(&setup_output.coverage);
        info!(harness = %test_harness.id(), address = %address, "setup executed");
    }

    // 10. Load the persisted corpus so mutations start from known sequences.
    //     The compilation output feeds literal extraction for argument
    //     generation.
    let corpus = Corpus::new()
        .with_root(&root)
        .with_dir(&args.corpus_dir)
        .with_harness(&args.harness)
        .with_handlers(test_harness.handlers().to_vec())
        .with_solc_output(&solc_output);
    info!(
        path = %strip_dot_prefix(corpus.path()?.display().to_string()),
        "loading corpus"
    );
    let loaded = corpus.load()?;
    info!(entries = loaded, "replaying corpus");

    // 11. Replay the loaded corpus so the fuzzers start from the coverage
    //     the sequences bring. Entries that no longer execute cleanly are
    //     dropped.
    let seed = jiff::Timestamp::now().as_nanosecond() as u64;
    let deployer = chain.deployer();
    let (corpus, _) = Replayer::new()
        .with_chain(chain.clone())
        .with_target(address)
        .with_deployer(deployer)
        .with_coverage(coverage.clone())
        .replay(corpus)?;
    info!("corpus loaded & replayed");

    // 12. Fuzz for broken invariants within the stop conditions.
    let shared_broken_invariants = SharedBrokenInvariants::new(args.max_failures);
    let fuzzer = Fuzzer::new()
        .with_chain(chain.clone())
        .with_target(address)
        .with_deployer(deployer)
        .with_handlers(test_harness.handlers().to_vec())
        .with_invariants(test_harness.invariants().to_vec())
        .with_corpus(corpus.clone())
        .with_coverage(coverage)
        .with_broken_invariants(shared_broken_invariants)
        .with_threads(args.threads)
        .with_max_runs(args.max_runs)
        .with_max_calls(args.max_calls)
        .with_timeout(args.timeout.map(Duration::from_secs))
        .with_seed(seed);
    let output = match fuzzer.run() {
        Ok(output) => output,
        Err(err) => {
            // Best-effort save so the corpus survives a failed campaign.
            if let Err(save_err) = corpus.save() {
                warn!(error = %save_err, "corpus save failed");
            }
            return Err(err);
        }
    };

    // 13. Shrink every broken invariant's sequence while the broken invariant
    //     still reproduces.
    let mut broken_invariants = output.broken_invariants;
    if !broken_invariants.is_empty() {
        broken_invariants = Shrinker::new()
            .with_chain(chain.clone())
            .with_target(address)
            .with_deployer(deployer)
            .with_threads(args.threads)
            .with_max_runs(args.max_runs)
            .with_timeout(args.timeout.map(Duration::from_secs))
            .with_seed(seed)
            .shrink(&broken_invariants)?;
    }

    // 14. Save the corpus for the next campaign.
    corpus.save()?;
    info!(
        entries = corpus.len(),
        path = %strip_dot_prefix(corpus.path()?.display().to_string()),
        "corpus saved"
    );

    // 15. Re-run every broken invariant with tracing so the console shows the
    //     logs emitted on the way to the broken invariant, and the trace file
    //     captures the full sequence.
    //
    //     The trigger call runs last and its state is discarded, so the
    //     optional summary call below still reports on the pre-trigger state.
    for broken in &broken_invariants {
        report_broken_invariant(
            &root,
            &chain,
            &trace_context,
            address,
            broken,
            test_harness.summary(),
        )?;
    }

    // 16. Run the summary function when no broken invariant was found so the
    //     campaign still reports its final state.
    if broken_invariants.is_empty()
        && let Some(summary) = test_harness.summary()
    {
        let summary_calldata = Bytes::from(summary.selector().as_slice().to_vec());
        let mut summary_chain = chain.clone();
        summary_chain.set_trace(true);
        let summary_tx = Transaction::new(address).calldata(summary_calldata);
        let summary_output = summary_chain.exec(std::slice::from_ref(&summary_tx))?;
        let trace = summary_output.trace.context("summary call trace missing")?;
        info!("\n{}", trace.display_logs_with(&trace_context));
        let trace_file = dump_execution_trace(&root, &trace_context, &trace)?;
        info!(
            harness = %test_harness.id(),
            path = %trace_file.display(),
            "execution trace saved"
        );
    }

    Ok(broken_invariants)
}

/// Re-run one broken invariant on a traced chain clone, print its logs, and
/// save the execution trace.
///
/// The transaction batch is the shrunk sequence, whose last call is the
/// bail-emitting trigger, plus the optional summary call.
fn report_broken_invariant(
    root: &Path,
    chain: &Chain,
    trace_context: &TraceContext,
    address: alloy_primitives::Address,
    broken: &BrokenInvariant,
    summary: Option<&alloy_json_abi::Function>,
) -> Result<()> {
    // 1. Build the traced re-run with the sequence and the summary.
    let deployer = chain.deployer();
    let mut rerun_chain = chain.clone();
    rerun_chain.set_trace(true);
    let mut transactions: Vec<Transaction> = broken.sequence().transactions(address, deployer);
    if let Some(summary) = summary {
        transactions.push(
            Transaction::new(address).calldata(Bytes::from(summary.selector().as_slice().to_vec())),
        );
    }

    // 2. Execute the re-run; the bail-emitting trigger is expected and does
    //    not invalidate the logs of the calls before it.
    let output = rerun_chain.exec(&transactions)?;

    // 3. Save the execution trace for offline analysis.
    let trace = output
        .trace
        .context("broken invariant re-run trace missing")?;
    let trace_file = dump_execution_trace(root, trace_context, &trace)?;
    info!(id = %broken.id(), trace = %trace_file.display(), "broken invariant saved");
    Ok(())
}

/// Dump an execution trace and return its absolute path.
///
/// The trace is written to
/// `{root}/.ripfuzz/traces/{unix-timestamp}-{id}.log`.
fn dump_execution_trace(
    root: &Path,
    trace_context: &TraceContext,
    trace: &Trace,
) -> Result<PathBuf> {
    // 1. Write the execution trace to a timestamped trace file.
    let trace_dir = root.join(".ripfuzz").join("traces");
    fs::create_dir_all(&trace_dir)?;
    let timestamp = jiff::Timestamp::now().as_second();
    let trace_file = trace_dir.join(format!("{timestamp}-{}.log", trace_id()));
    let trace = trace.display_with(trace_context).to_string();
    fs::write(&trace_file, trace)
        .with_context(|| format!("failed to write {}", trace_file.display()))?;

    // 2. Return the absolute path so logs and errors can point at the file.
    Ok(absolute(trace_file)?)
}

/// Short unique id for a trace file name.
fn trace_id() -> String {
    let uuid: String = uuid::Uuid::new_v4().into();
    uuid.split('-').next().unwrap_or_default().to_owned()
}

fn strip_dot_prefix(path: impl AsRef<Path>) -> String {
    let mut display = path.as_ref().display().to_string();
    loop {
        if let Some(stripped) = display.strip_prefix("./") {
            display = stripped.to_owned();
        } else if let Some(stripped) = display.strip_prefix(".\\") {
            display = stripped.to_owned();
        } else {
            break;
        }
    }
    display
}
