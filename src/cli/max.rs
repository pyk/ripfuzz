//! `max` CLI command implementation.

use std::fs;
use std::path::{Path, PathBuf, absolute};

use anyhow::{Context, Result, bail};
use clap::Parser;
use tracing::{error, info};

use crate::config::Config;
use crate::evm::{Chain, ChainConfig, DeployOutput, TraceContext};
use crate::harness::HarnessId;
use crate::max::MaxHarness;
use crate::solc::Solc;

/// Maximize a harness value.
#[derive(Debug, Parser)]
pub struct Args {
    /// Harness to maximize.
    #[arg(value_name = "HARNESS")]
    pub harness: HarnessId,

    /// Path to the ripfuzz config file.
    #[arg(long, default_value = "ripfuzz.toml", value_name = "PATH")]
    pub config: PathBuf,

    /// Project root directory.
    #[arg(long, value_name = "PATH")]
    pub root: Option<PathBuf>,
}

/// Run the `max` command.
pub fn run(args: Args) -> Result<()> {
    // 1. Initialize tracing subscriber.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();

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
    let chain_config = ChainConfig::new(&root).coverage(true);
    let mut chain = Chain::new(chain_config)?;

    // 6. Deploy the harness contract.
    let deployment = chain.deploy(&max_harness)?;
    if !deployment.result.success {
        // 6a. Label the trace context from the compilation output and chain.
        let mut trace_context = TraceContext::from(&solc_output);
        let labels = chain.labels().clone();
        for (address, label) in labels {
            trace_context = trace_context.with_label(address, label);
        }

        // 6b. Dump the execution trace and point at it from the logs.
        let trace_file = dump_failed_deployment_trace(&root, &trace_context, &deployment)?;
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
    println!("{address}");
    Ok(())
}

/// Dump the execution trace of a failed deployment and return its absolute
/// path.
///
/// The trace is written to
/// `{root}/.ripfuzz/traces/{unix-timestamp}-{id}.log`.
fn dump_failed_deployment_trace(
    root: &Path,
    trace_context: &TraceContext,
    deployment: &DeployOutput,
) -> Result<PathBuf> {
    // 1. Write the execution trace to a timestamped trace file.
    let trace_dir = root.join(".ripfuzz").join("traces");
    fs::create_dir_all(&trace_dir)?;
    let timestamp = jiff::Timestamp::now().as_second();
    let trace_file = trace_dir.join(format!("{timestamp}-{}.log", trace_id()));
    let trace = deployment.trace.display_with(trace_context).to_string();
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
