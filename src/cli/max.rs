//! `max` CLI command implementation.

use std::fs;
use std::path::{Path, PathBuf, absolute};
use std::time::Duration;

use alloy_primitives::U256;
use anyhow::{Context, Result, bail};
use clap::Parser;
use revm::primitives::Bytes;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::evm::{
    Chain, ChainConfig, ForkDBConfig, SetupInput, SharedCoverage, Trace, TraceContext, Transaction,
};
use crate::harness::HarnessId;
use crate::max::{Best, Corpus, CorpusReplayer, Fuzzer, MaxHarness, Shrinker, Value};
use crate::solc::Solc;

/// Maximize a harness value.
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

    /// Stop fuzzing when the target value is reached.
    #[arg(long, value_name = "VALUE", value_parser = parse_u256)]
    pub target_value: Option<U256>,

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

/// Parse a `uint256` CLI value in decimal or `0x`-prefixed hex.
fn parse_u256(value: &str) -> Result<U256, String> {
    value.parse::<U256>().map_err(|err| err.to_string())
}

/// Run the `max` command and return the best sequence found.
pub fn run(args: Args) -> Result<Best> {
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
        .with_version(&config.solc)
        .with_root(&root)
        .with_target(&args.harness.path)
        .with_name(&args.harness.name)
        .with_out(&config.out)
        .compile()?;

    // 4. Validate the compiled output against the max harness rules.
    let max_harness = MaxHarness::try_from(&solc_output)?;

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

    // 7. Deploy the harness contract.
    let deployment = chain.deploy(&max_harness)?;
    if !deployment.result.success {
        let trace_file = dump_execution_trace(&root, &trace_context, &deployment.trace)?;
        error!("execution trace: {}", trace_file.display());
        bail!(
            "harness contract `{}` deployment failed",
            max_harness.id().name
        );
    }
    let address = deployment
        .address
        .context("deployment succeeded but created_address is missing")?;
    info!(harness = %max_harness.id(), address = %address, "harness deployed");

    // 8. Run the setup function if the harness defines one.
    // checkrs: allow(nested_if_let)
    if let Some(setup) = max_harness.setup() {
        let setup_input =
            SetupInput::new(address).calldata(Bytes::from(setup.selector().as_slice().to_vec()));
        let setup_output = chain.setup(setup_input)?;
        if !setup_output.result.success {
            let trace_file = dump_execution_trace(&root, &trace_context, &setup_output.trace)?;
            error!("execution trace: {}", trace_file.display());
            bail!("harness contract `{}` setup failed", max_harness.id().name);
        }
        info!(harness = %max_harness.id(), address = %address, "setup executed");
    }

    // 9. Measure the initial value reported by the harness.
    //
    //    The call runs on a traced chain clone because `Chain::call`
    //    returns no execution trace.
    let value_calldata = Bytes::from(max_harness.value().selector().as_slice().to_vec());
    let mut value_chain = chain.clone();
    value_chain.set_trace(true);
    let value_tx = Transaction::new(address).calldata(value_calldata.clone());
    let value_output = value_chain.exec(std::slice::from_ref(&value_tx))?;
    let value_result = value_output
        .results
        .first()
        .context("value call result missing")?;
    if !value_result.success {
        let trace = value_output.trace.context("value call trace missing")?;
        let trace_file = dump_execution_trace(&root, &trace_context, &trace)?;
        error!("execution trace: {}", trace_file.display());
        bail!(
            "harness contract `{}` value call failed",
            max_harness.id().name
        );
    }
    let initial_value = Value::decode(value_result).with_context(|| {
        format!(
            "harness contract `{}` returned an invalid value",
            max_harness.id()
        )
    })?;
    info!(
        harness = %max_harness.id(),
        initial_value = %initial_value,
        "initial value measured"
    );

    // 10. Load the persisted corpus so mutations start from known sequences.
    let corpus_path = corpus_path(&root, &args.corpus_dir, &args.harness)?;
    let corpus = Corpus::new();
    let loaded = corpus.load(&corpus_path, &max_harness.handlers())?;
    info!(
        entries = loaded,
        path = %corpus_path.display(),
        "corpus loaded"
    );

    // 11. Replay the loaded corpus so the fuzzers start from the coverage
    //     the sequences bring and from values re-measured on the current
    //     harness. Entries that no longer execute cleanly are dropped.
    let seed = jiff::Timestamp::now().as_nanosecond() as u64;
    let deployer = chain.deployer();
    let coverage = SharedCoverage::new();
    let (corpus, replayed) = CorpusReplayer::new()
        .with_chain(chain.clone())
        .with_target(address)
        .with_deployer(deployer)
        .with_value_calldata(value_calldata.clone())
        .with_coverage(coverage.clone())
        .replay(corpus)?;
    info!(entries = replayed, "corpus replayed");

    // 12. Fuzz for the highest value within the stop conditions.
    let fuzzer = Fuzzer::new()
        .with_chain(chain.clone())
        .with_target(address)
        .with_deployer(deployer)
        .with_value_calldata(value_calldata.clone())
        .with_handlers(max_harness.handlers())
        .with_corpus(corpus.clone())
        .with_coverage(coverage)
        .with_initial_value(initial_value)
        .with_threads(args.threads)
        .with_max_runs(args.max_runs)
        .with_max_calls(args.max_calls)
        .with_timeout(args.timeout.map(Duration::from_secs))
        .with_target_value(args.target_value.map(Value::new))
        .with_seed(seed);
    let best = match fuzzer.run() {
        Ok(best) => best,
        Err(err) => {
            // Best-effort save so the corpus survives a failed campaign.
            if let Err(save_err) = corpus.save(&corpus_path) {
                warn!(error = %save_err, "corpus save failed");
            }
            return Err(err);
        }
    };

    // 13. Shrink the best sequence while preserving its value, and keep the
    //     minimal sequence in the corpus so the next campaign starts from
    //     the shortest sequence that reaches the best value.
    if !best.sequence().is_empty() {
        let shrunk = Shrinker::new()
            .with_chain(chain.clone())
            .with_target(address)
            .with_deployer(deployer)
            .with_value_calldata(value_calldata)
            .with_target_value(best.value())
            .with_threads(args.threads)
            .with_max_runs(args.max_runs)
            .with_timeout(args.timeout.map(Duration::from_secs))
            .with_seed(seed)
            .shrink(best.sequence())?;
        info!(
            calls = shrunk.len(),
            sequence = %shrunk,
            "shrunk best sequence"
        );

        // The shrunk sequence brings no new coverage of its own, so it
        // inherits the edge count of the best sequence it was shrunk from,
        // keeping it competitive during eviction.
        let new_edges = corpus
            .entries()
            .iter()
            .find(|entry| entry.sequence == *best.sequence())
            .map(|entry| entry.new_edges)
            .unwrap_or(0);

        // Re-execute the shrunk sequence so the corpus entry carries the
        // state after it and the next campaign can expand from it.
        let mut shrunk_chain = chain;
        let transactions = shrunk.transactions(address, deployer);
        shrunk_chain.exec(&transactions)?;
        corpus.add(shrunk, best.value(), new_edges, shrunk_chain);
    }

    // 14. Save the corpus for the next campaign.
    corpus.save(&corpus_path)?;
    info!(
        entries = corpus.len(),
        path = %corpus_path.display(),
        "corpus saved"
    );

    Ok(best)
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
