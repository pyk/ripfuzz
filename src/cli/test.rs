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
use crate::test::{Corpus, Finding, Fuzzer, Replayer, SharedFindings, Shrinker, TestHarness};

/// Find failed assertions.
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

    /// Stop fuzzing after this many distinct failed assertions.
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

/// Run the `test` command and return the shrunk findings.
pub fn run(args: Args) -> Result<Vec<Finding>> {
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
    info!(harness = %test_harness.id(), address = %address, "harness deployed");

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
    let corpus_path = corpus_path(&root, &args.corpus_dir, &args.harness)?;
    let corpus = Corpus::new();
    let loaded = corpus.load(&corpus_path, test_harness.handlers())?;
    info!(
        entries = loaded,
        path = %corpus_path.display(),
        "corpus loaded"
    );

    // 11. Replay the loaded corpus so the fuzzers start from the coverage
    //     the sequences bring. Entries that no longer execute cleanly are
    //     dropped.
    let seed = jiff::Timestamp::now().as_nanosecond() as u64;
    let deployer = chain.deployer();
    let (corpus, replayed) = Replayer::new()
        .with_chain(chain.clone())
        .with_target(address)
        .with_deployer(deployer)
        .with_coverage(coverage.clone())
        .replay(corpus)?;
    info!(entries = replayed, "corpus replayed");

    // 12. Fuzz for failed assertions within the stop conditions.
    let shared_findings = SharedFindings::new(args.max_failures);
    let fuzzer = Fuzzer::new()
        .with_chain(chain.clone())
        .with_target(address)
        .with_deployer(deployer)
        .with_handlers(test_harness.handlers().to_vec())
        .with_invariants(test_harness.invariants().to_vec())
        .with_corpus(corpus.clone())
        .with_coverage(coverage)
        .with_findings(shared_findings)
        .with_threads(args.threads)
        .with_max_runs(args.max_runs)
        .with_max_calls(args.max_calls)
        .with_timeout(args.timeout.map(Duration::from_secs))
        .with_seed(seed);
    let output = match fuzzer.run() {
        Ok(output) => output,
        Err(err) => {
            // Best-effort save so the corpus survives a failed campaign.
            if let Err(save_err) = corpus.save(&corpus_path) {
                warn!(error = %save_err, "corpus save failed");
            }
            return Err(err);
        }
    };

    // 13. Shrink every finding's sequence while the assertion still panics.
    let mut findings = output.findings;
    if !findings.is_empty() {
        findings = Shrinker::new()
            .with_chain(chain.clone())
            .with_target(address)
            .with_deployer(deployer)
            .with_threads(args.threads)
            .with_max_runs(args.max_runs)
            .with_timeout(args.timeout.map(Duration::from_secs))
            .with_seed(seed)
            .shrink(&findings)?;
    }

    // 14. Save the corpus for the next campaign.
    corpus.save(&corpus_path)?;
    info!(
        entries = corpus.len(),
        path = %corpus_path.display(),
        "corpus saved"
    );

    // 15. Re-run every finding with tracing so the console shows the logs
    //     emitted on the way to the assertion, and the trace file captures
    //     the full sequence.
    //
    //     The trigger call runs last and its state is discarded, so the
    //     optional summary call below still reports on the pre-failure
    //     state.
    for finding in &findings {
        report_finding(
            &root,
            &chain,
            &trace_context,
            address,
            finding,
            test_harness.summary(),
        )?;
    }

    // 16. Run the summary function when no assertion failed so the campaign
    //     still reports its final state.
    if findings.is_empty()
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

    Ok(findings)
}

/// Re-run one finding on a traced chain clone, print its logs, and save the
/// execution trace.
///
/// The transaction batch is the shrunk sequence, the trigger call that must
/// panic, and the optional summary call.
fn report_finding(
    root: &Path,
    chain: &Chain,
    trace_context: &TraceContext,
    address: alloy_primitives::Address,
    finding: &Finding,
    summary: Option<&alloy_json_abi::Function>,
) -> Result<()> {
    // 1. Build the traced re-run with the sequence, trigger, and summary.
    let deployer = chain.deployer();
    let mut rerun_chain = chain.clone();
    rerun_chain.set_trace(true);
    let mut transactions: Vec<Transaction> = finding.sequence().transactions(address, deployer);
    transactions.push(Transaction::new(address).calldata(Bytes::from(
        finding.trigger().selector().as_slice().to_vec(),
    )));
    if let Some(summary) = summary {
        transactions.push(
            Transaction::new(address).calldata(Bytes::from(summary.selector().as_slice().to_vec())),
        );
    }

    // 2. Execute the re-run; the trigger panic is expected and does not
    //    invalidate the logs of the calls before it.
    let output = rerun_chain.exec(&transactions)?;

    // 3. Show the re-run logs in the console.
    let trace = output.trace.context("finding re-run trace missing")?;
    info!(
        function = %finding.trigger().signature(),
        reason = %finding.reason_display(),
        calls = finding.sequence().len(),
        sequence = %finding.sequence(),
        "\n{}",
        trace.display_logs_with(trace_context)
    );

    // 4. Save the execution trace for offline analysis.
    let trace_file = dump_execution_trace(root, trace_context, &trace)?;
    info!(path = %trace_file.display(), "execution trace saved");
    Ok(())
}

/// Resolve the corpus file path for a harness.
///
/// Relative corpus directories resolve against the project root, mirroring
/// the solc out dir. The file is namespaced by the harness source file and
/// contract name, mirroring the compilation output layout, so targets
/// sharing a corpus directory never overwrite each other's `corpus.json`.
fn corpus_path(
    root: impl AsRef<Path>,
    corpus_dir: impl AsRef<Path>,
    harness: &HarnessId,
) -> Result<PathBuf> {
    // 1. Resolve the corpus directory relative to the project root.
    let corpus_dir = corpus_dir.as_ref();
    let base = if corpus_dir.is_absolute() {
        corpus_dir.to_path_buf()
    } else {
        root.as_ref().join(corpus_dir)
    };

    // 2. Namespace the file by the source file and contract name.
    let file_name = harness
        .path
        .file_name()
        .context("harness path has no file name")?;
    Ok(base.join(file_name).join(&harness.name).join("corpus.json"))
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
