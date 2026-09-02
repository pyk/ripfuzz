//! `max` CLI command implementation.

use std::path::{Path, PathBuf};
use std::time::Duration;

use alloy_primitives::U256;
use anyhow::{Context, Result, bail};
use clap::Parser;
use revm::primitives::Bytes;
use tracing::{error, info, warn};

use crate::compilers::solc::Solc;
use crate::config::Config;
use crate::evm::{
    Chain, ChainConfig, CoverageReporter, CoverageWriter, ExecutionTraceWriter, ForkDBConfig,
    SetupInput, SharedCoverage, TraceContext, Transaction,
};
use crate::harness::HarnessId;
use crate::logger::Logger;
use crate::maxer::{Best, Corpus, CorpusReplayer, Fuzzer, MaxHarness, Sequence, Shrinker, Value};

/// Find maximum value.
#[derive(Debug, Parser)]
pub struct Command {
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
    #[arg(long, default_value_t = crate::cli::default_threads(), value_name = "THREADS")]
    pub threads: usize,

    /// Maximum number of sequences to fuzz, split across all threads.
    #[arg(long, default_value_t = 100_000, value_name = "RUNS")]
    pub max_fuzz_runs: u64,

    /// Maximum number of shrink attempts, split across all threads.
    #[arg(long, default_value_t = 10_000, value_name = "RUNS")]
    pub max_shrink_runs: u64,

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

impl Command {
    /// Run the `max` command and return the best sequence found.
    pub fn run(&self) -> Result<Best> {
        // 1. Initialize logging. Quiet mode writes the file layer only, so a
        //    subscriber installed by an earlier caller (e.g. a test binary)
        //    cannot leak events into the terminal.
        Logger::new()
            .with_root(&self.root)
            .with_quiet(self.quiet)
            .with_level(self.log_level)
            .init()?;

        // 2. Load configuration relative to the project root.
        let root = self.root.clone();
        let config = Config::new().with_root(&root).load(&self.config)?;

        // 3. Compile the harness via Solc relative to the project root.
        let solc_output = Solc::new()
            .with_version(&config.solc.version)
            .with_root(&root)
            .with_target(&self.harness.path)
            .with_name(&self.harness.name)
            .with_out(&config.solc.out)
            .with_evm_version(config.solc.evm_version)
            .with_optimizer(config.solc.optimizer, config.solc.optimizer_runs)
            .with_via_ir(config.solc.via_ir)
            .with_remappings(config.solc.remappings.clone())
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

        // 6. Label the trace context from the compilation output and the chain,
        //    and create the writer that saves execution traces under the root.
        let mut trace_context = TraceContext::from(&solc_output);
        let labels = chain.labels().clone();
        for (address, label) in labels {
            trace_context = trace_context.with_label(address, label);
        }
        let trace_writer =
            ExecutionTraceWriter::new(&root).with_trace_context(trace_context.clone());

        // 7. Create the shared coverage map so the deployment and setup calls
        //    below seed the baseline the fuzzers measure new edges against.
        let coverage = SharedCoverage::new();

        // 8. Deploy the harness contract.
        info!("deploying harness");
        let deployment = chain.deploy(&max_harness)?;
        if !deployment.result.success {
            let trace_file = trace_writer.write(&deployment.trace)?;
            error!("execution trace: {}", trace_file.display());
            bail!(
                "harness contract `{}` deployment failed",
                max_harness.id().name
            );
        }
        let address = deployment
            .address
            .context("deployment succeeded but created_address is missing")?;
        coverage.merge(&deployment.coverage);
        info!("harness deployed at {address}");

        // 9. Run the setup function if the harness defines one.
        // checkrs: allow(nested_if_let)
        if let Some(setup) = max_harness.setup() {
            let setup_input = SetupInput::new(address)
                .calldata(Bytes::from(setup.selector().as_slice().to_vec()));
            let setup_output = chain.setup(setup_input)?;
            if !setup_output.result.success {
                let trace_file = trace_writer.write(&setup_output.trace)?;
                error!("execution trace: {}", trace_file.display());
                bail!("harness contract `{}` setup failed", max_harness.id().name);
            }
            coverage.merge(&setup_output.coverage);
            info!("setup executed for {} at {address}", max_harness.id());
        }

        // 10. Measure the initial value reported by the harness.
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
            let trace_file = trace_writer.write(&trace)?;
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
            "initial value {initial_value} measured for {}",
            max_harness.id()
        );

        // 11. Load the persisted corpus so mutations start from known sequences.
        let corpus_path = corpus_path(&root, &self.corpus_dir, &self.harness)?;
        let corpus = Corpus::new();
        info!(
            "loading corpus {}",
            strip_dot_prefix(corpus_path.display().to_string())
        );
        let loaded = corpus.load(&corpus_path, &max_harness.handlers())?;
        let entries = match loaded {
            1 => "1 corpus entry".to_string(),
            n => format!("{n} corpus entries"),
        };
        info!("replaying {entries}");

        // 12. Replay the loaded corpus so the fuzzers start from the coverage
        //     the sequences bring and from values re-measured on the current
        //     harness. Entries that no longer execute cleanly are dropped.
        let seed = jiff::Timestamp::now().as_nanosecond() as u64;
        let deployer = chain.deployer();
        let (corpus, _) = CorpusReplayer::new()
            .with_chain(chain.clone())
            .with_target(address)
            .with_deployer(deployer)
            .with_value_calldata(value_calldata.clone())
            .with_coverage(coverage.clone())
            .replay(corpus)?;
        info!("corpus loaded & replayed");

        // 13. Fuzz for the highest value within the stop conditions.
        let fuzzer = Fuzzer::new()
            .with_chain(chain.clone())
            .with_target(address)
            .with_deployer(deployer)
            .with_value_calldata(value_calldata.clone())
            .with_handlers(max_harness.handlers())
            .with_corpus(corpus.clone())
            .with_coverage(coverage.clone())
            .with_initial_value(initial_value)
            .with_threads(self.threads)
            .with_max_runs(self.max_fuzz_runs)
            .with_max_calls(self.max_calls)
            .with_timeout(self.timeout.map(Duration::from_secs))
            .with_target_value(self.target_value.map(Value::new))
            .with_seed(seed);
        let best = match fuzzer.run() {
            Ok(best) => best,
            Err(err) => {
                // Best-effort save so the corpus survives a failed campaign.
                if let Err(save_err) = corpus.save(&corpus_path) {
                    warn!("corpus save failed: {save_err:#}");
                }
                return Err(err);
            }
        };

        // 14. Shrink the best sequence while preserving its value, and keep the
        //     minimal sequence in the corpus so the next campaign starts from
        //     the shortest sequence that reaches the best value.
        //
        //     `final_chain` carries the state after the best sequence so the
        //     summary below reports on the campaign outcome.
        let mut final_chain = chain.clone();
        let mut final_sequence: Option<Sequence> = None;
        if !best.sequence().is_empty() {
            let shrunk = Shrinker::new()
                .with_chain(chain.clone())
                .with_target(address)
                .with_deployer(deployer)
                .with_value_calldata(value_calldata)
                .with_target_value(best.value())
                .with_threads(self.threads)
                .with_max_runs(self.max_shrink_runs)
                .with_timeout(self.timeout.map(Duration::from_secs))
                .with_seed(seed)
                .shrink(best.sequence())?;
            info!(
                "best sequence {} minimized from {} calls to {}",
                best.value(),
                best.sequence().len(),
                shrunk.len(),
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
            let transactions = shrunk.transactions(address, deployer);
            final_chain.exec(&transactions)?;
            corpus.add(
                shrunk.clone(),
                best.value(),
                new_edges,
                0,
                final_chain.clone(),
            );
            final_sequence = Some(shrunk);
        }

        // 15. Save the corpus for the next campaign.
        corpus.save(&corpus_path)?;
        let entries = match corpus.len() {
            1 => "1 entry".to_string(),
            n => format!("{n} entries"),
        };
        info!("corpus saved: {entries} to {}", corpus_path.display());

        // 16. Write the campaign coverage report.
        let report = CoverageReporter::new()
            .solc_output(&solc_output)
            .shared_coverage(coverage)
            .base_project_path(&root)
            .build();
        let coverage_file = CoverageWriter::new(&root).write(&report)?;
        info!(
            "coverage report saved to {}",
            strip_dot_prefix(coverage_file.display().to_string())
        );

        // 17. Run the summary function if the harness defines one.
        //
        //     The call runs on a traced clone of the base chain with the best
        //     sequence replayed so the console and the saved trace show the full
        //     re-run that leads to the profit breakdown after the best sequence.
        //     A failing summary must not discard the best sequence, so it only
        //     warns.
        let Some(summary) = max_harness.summary() else {
            return Ok(best);
        };
        let summary_calldata = Bytes::from(summary.selector().as_slice().to_vec());
        let summary_tx = Transaction::new(address).calldata(summary_calldata);
        // 17a. Build the traced re-run with the best sequence followed by summary.
        let mut summary_chain = chain.clone();
        summary_chain.set_trace(true);
        let mut summary_txs = Vec::new();
        if let Some(sequence) = final_sequence.as_ref() {
            summary_txs.extend(sequence.transactions(address, deployer));
        }
        summary_txs.push(summary_tx);
        let summary_output = summary_chain.exec(&summary_txs)?;
        let summary_result = summary_output
            .results
            .last()
            .context("summary call result missing")?;
        if !summary_result.success {
            warn!("summary call failed for {}", max_harness.id());
        }
        // 17b. Show the summary logs in the console.
        let trace = summary_output.trace.context("summary call trace missing")?;
        info!("\n{}", trace.display_logs_with(&trace_context));

        let trace_file = trace_writer.write(&trace)?;
        info!(
            "execution trace for {} saved to {}",
            max_harness.id(),
            trace_file.display()
        );

        Ok(best)
    }
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
