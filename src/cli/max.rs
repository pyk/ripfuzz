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
use crate::max::{Best, Corpus, Fuzzer, FuzzerConfig, MaxHarness, Shrinker, ShrinkerConfig, Value};
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
    #[arg(long, value_name = "PATH")]
    pub root: Option<PathBuf>,

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

    /// Directory to dump the corpus at the end of the campaign.
    #[arg(long, value_name = "PATH")]
    pub corpus_dir: Option<PathBuf>,

    /// Suppress terminal log output.
    #[arg(short = 'q', long)]
    pub quiet: bool,
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
        .with_max_level(tracing::Level::INFO)
        .with_target(false);
    let _ = if args.quiet {
        builder.with_writer(std::io::sink).try_init()
    } else {
        builder.try_init()
    };

    // 2. Load configuration relative to the project root.
    let root = args.root.clone().unwrap_or_else(|| PathBuf::from("."));
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

    // 10. Fuzz for the highest value within the stop conditions.
    let seed = jiff::Timestamp::now().as_nanosecond() as u64;
    let deployer = chain.deployer();
    let corpus = Corpus::new();
    let fuzzer_config = FuzzerConfig::new()
        .chain(chain.clone())
        .target(address)
        .deployer(deployer)
        .value_calldata(value_calldata.clone())
        .handlers(max_harness.handlers())
        .corpus(corpus.clone())
        .coverage(SharedCoverage::new())
        .initial_value(initial_value)
        .threads(args.threads)
        .max_runs(args.max_runs)
        .max_calls(args.max_calls)
        .timeout(args.timeout.map(Duration::from_secs))
        .target_value(args.target_value.map(Value::new))
        .seed(seed);
    let fuzzer = Fuzzer::new(fuzzer_config);
    let corpus_file = corpus_dump_path(&root, &args.corpus_dir, &args.harness)?;
    let best = match fuzzer.run() {
        Ok(best) => {
            dump_corpus(&corpus, &corpus_file)?;
            best
        }
        Err(err) => {
            // Best-effort dump so the corpus survives a failed campaign.
            if let Err(dump_err) = dump_corpus(&corpus, &corpus_file) {
                warn!(error = %dump_err, "corpus dump failed");
            }
            return Err(err);
        }
    };

    // 11. Shrink the best sequence while preserving its value.
    if !best.sequence().is_empty() {
        let shrinker_config = ShrinkerConfig::new()
            .chain(chain)
            .target(address)
            .deployer(deployer)
            .value_calldata(value_calldata)
            .target_value(best.value())
            .threads(args.threads)
            .max_runs(args.max_runs)
            .timeout(args.timeout.map(Duration::from_secs))
            .seed(seed);
        let shrinker = Shrinker::new(shrinker_config);
        let shrunk = shrinker.shrink(best.sequence())?;
        info!(
            calls = shrunk.len(),
            sequence = %shrunk,
            "shrunk best sequence"
        );
    }

    println!("{address}");
    Ok(best)
}

/// Resolve the corpus dump file path for a harness.
///
/// The dump defaults to `{root}/.ripfuzz/corpus` and is namespaced by the
/// harness source file and contract name, mirroring the compilation output
/// layout, so targets sharing a corpus directory never overwrite each
/// other's dumps.
fn corpus_dump_path(
    root: impl AsRef<Path>,
    corpus_dir: &Option<PathBuf>,
    harness: &HarnessId,
) -> Result<PathBuf> {
    // 1. Resolve the corpus base directory relative to the project root.
    let base = corpus_dir
        .clone()
        .unwrap_or_else(|| root.as_ref().join(".ripfuzz").join("corpus"));

    // 2. Namespace the dump by the source file and contract name.
    let file_name = harness
        .path
        .file_name()
        .context("harness path has no file name")?;
    Ok(base.join(file_name).join(&harness.name).join("corpus.log"))
}

/// Dump the corpus of interesting sequences and return the written path.
///
/// Each entry is one line with its value, new coverage, call count, and
/// sequence, in corpus order so the file also reflects the eviction
/// history.
fn dump_corpus(corpus: &Corpus, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    // 1. Render one line per entry.
    let mut dump = String::new();
    for entry in corpus.entries() {
        dump.push_str(&format!(
            "value={} edges={} calls={} sequence={}\n",
            entry.value,
            entry.new_edges,
            entry.sequence.len(),
            entry.sequence
        ));
    }

    // 2. Write the dump under its namespaced directory.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, &dump).with_context(|| format!("failed to write {}", path.display()))?;
    info!(entries = corpus.len(), path = %path.display(), "corpus dumped");
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
