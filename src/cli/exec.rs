//! `exec` CLI command implementation.

use std::fs;
use std::path::{Path, PathBuf, absolute};

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use revm::primitives::Bytes;
use tracing::{error, info};

use crate::config::Config;
use crate::evm::{Chain, ChainConfig, ForkDBConfig, SetupInput, Trace, TraceContext, Transaction};
use crate::exec::Script;
use crate::harness::HarnessId;
use crate::solc::Solc;

/// Execute a script contract.
#[derive(Debug, Parser)]
pub struct Args {
    /// Path to script to run.
    #[arg(value_name = "SCRIPT")]
    pub script: HarnessId,

    /// Path to the ripfuzz config file.
    #[arg(long, default_value = "ripfuzz.toml", value_name = "PATH")]
    pub config: PathBuf,

    /// Project root directory.
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub root: PathBuf,

    /// Suppress terminal log output.
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Log verbosity level.
    #[arg(long, default_value = "info", value_name = "LEVEL")]
    pub log_level: tracing::Level,
}

/// Run the `exec` command.
pub fn run(args: Args) -> Result<()> {
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

    // 3. Ensure the script file exists relative to the project root.
    let script_path = root.join(&args.script.path);
    ensure!(
        script_path.is_file(),
        "script file `{}` not found",
        args.script.path.display()
    );

    // 4. Compile the script via Solc relative to the project root.
    let solc_output = Solc::new()
        .with_version(&config.solc)
        .with_root(&root)
        .with_target(&args.script.path)
        .with_name(&args.script.name)
        .with_out(&config.out)
        .compile()?;

    // 5. Validate the compiled output against the exec script rules.
    let script = Script::try_from(&solc_output)?;

    // 6. Create the test chain the script will run on.
    //
    //    Forks share the on-disk RPC cache with other commands, and a
    //    conservative batch rate limit keeps default runs under
    //    public-provider quotas. Tracing stays enabled so the setup and exec
    //    calls capture logs and the execution trace.
    let fork_defaults = ForkDBConfig::new("")
        .cache_dir(root.join(".ripfuzz").join("cache"))
        .rate_limit(Some(10));
    let chain_config = ChainConfig::new(&root)
        .with_fork_defaults(fork_defaults)
        .trace(true);
    let mut chain = Chain::new(chain_config)?;

    // 7. Label the trace context from the compilation output and the chain.
    let mut trace_context = TraceContext::from(&solc_output);
    let labels = chain.labels().clone();
    for (address, label) in labels {
        trace_context = trace_context.with_label(address, label);
    }

    // 8. Deploy the script contract.
    let deployment = chain.deploy(&script)?;
    ensure_call_success(
        &root,
        &trace_context,
        deployment.result.success,
        &deployment.trace,
        &format!("script contract `{}` deployment failed", script.id().name),
    )?;
    let address = deployment
        .address
        .context("deployment succeeded but created_address is missing")?;
    info!(script = %script.id(), address = %address, "script deployed");

    // 9. Run the setup function if the script defines one.
    if let Some(setup) = script.setup() {
        let setup_input =
            SetupInput::new(address).calldata(Bytes::from(setup.selector().as_slice().to_vec()));
        let setup_output = chain.setup(setup_input)?;
        ensure_call_success(
            &root,
            &trace_context,
            setup_output.result.success,
            &setup_output.trace,
            &format!("script contract `{}` setup failed", script.id().name),
        )?;
        print_logs(&setup_output.trace, &trace_context);
        info!(script = %script.id(), address = %address, "setup executed");
    }

    // 10. Execute the exec function.
    let exec_calldata = Bytes::from(script.exec().selector().as_slice().to_vec());
    let exec_tx = Transaction::new(address).calldata(exec_calldata);
    let exec_output = chain.exec(std::slice::from_ref(&exec_tx))?;
    let exec_result = exec_output
        .results
        .first()
        .context("exec call result missing")?;
    let trace = exec_output.trace.context("exec call trace missing")?;
    ensure_call_success(
        &root,
        &trace_context,
        exec_result.success,
        &trace,
        &format!("script contract `{}` exec failed", script.id().name),
    )?;

    // 11. Print the script logs into the console.
    print_logs(&trace, &trace_context);

    // 12. Save the execution trace.
    let trace_file = dump_execution_trace(&root, &trace_context, &trace)?;
    info!(
        script = %script.id(),
        path = %trace_file.display(),
        "execution trace saved"
    );

    Ok(())
}

/// Bail with a dumped execution trace when a call failed.
fn ensure_call_success(
    root: &Path,
    trace_context: &TraceContext,
    success: bool,
    trace: &Trace,
    message: &str,
) -> Result<()> {
    if success {
        return Ok(());
    }
    let trace_file = dump_execution_trace(root, trace_context, trace)?;
    error!("execution trace: {}", trace_file.display());
    bail!("{message}");
}

/// Print the log output of an execution trace into the console.
///
/// Only decoded log entries render here, so custom events stay in the saved
/// execution trace. A trace without log output prints nothing.
fn print_logs(trace: &Trace, trace_context: &TraceContext) {
    let logs = trace.display_logs_with(trace_context).to_string();
    if !logs.trim().is_empty() {
        info!("\n{logs}");
    }
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
