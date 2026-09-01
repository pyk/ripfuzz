//! `exec` CLI command implementation.

use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use revm::primitives::Bytes;
use tracing::{error, info};

use crate::compilers::solc::Solc;
use crate::config::Config;
use crate::evm::{
    Chain, ChainConfig, ExecutionTraceWriter, ForkDBConfig, SetupInput, Trace, TraceContext,
    Transaction,
};
use crate::executor::Script;
use crate::harness::HarnessId;
use crate::logger::Logger;

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
    // 1. Initialize logging. Quiet mode writes the file layer only, so a
    //    subscriber installed by an earlier caller (e.g. a test binary)
    //    cannot leak events into the terminal.
    Logger::new(&args.root)
        .with_quiet(args.quiet)
        .with_level(args.log_level)
        .init()?;

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
        .with_version(&config.solc.version)
        .with_root(&root)
        .with_target(&args.script.path)
        .with_name(&args.script.name)
        .with_out(&config.solc.out)
        .with_evm_version(config.solc.evm_version)
        .with_optimizer(config.solc.optimizer, config.solc.optimizer_runs)
        .with_via_ir(config.solc.via_ir)
        .with_remappings(config.solc.remappings.clone())
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

    // 7. Label the trace context from the compilation output and the chain,
    //    and create the writer that saves execution traces under the root.
    let mut trace_context = TraceContext::from(&solc_output);
    let labels = chain.labels().clone();
    for (address, label) in labels {
        trace_context = trace_context.with_label(address, label);
    }
    let trace_writer = ExecutionTraceWriter::new(&root).with_trace_context(trace_context.clone());

    // 8. Deploy the script contract.
    let deployment = chain.deploy(&script)?;
    ensure_call_success(
        &trace_writer,
        deployment.result.success,
        &deployment.trace,
        &format!("script contract `{}` deployment failed", script.id().name),
    )?;
    let address = deployment
        .address
        .context("deployment succeeded but created_address is missing")?;
    info!("script {} deployed at {address}", script.id());

    // 9. Run the setup function if the script defines one.
    if let Some(setup) = script.setup() {
        let setup_input =
            SetupInput::new(address).calldata(Bytes::from(setup.selector().as_slice().to_vec()));
        let setup_output = chain.setup(setup_input)?;
        ensure_call_success(
            &trace_writer,
            setup_output.result.success,
            &setup_output.trace,
            &format!("script contract `{}` setup failed", script.id().name),
        )?;
        print_logs(&setup_output.trace, &trace_context);
        info!("setup executed for {} at {address}", script.id());
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
        &trace_writer,
        exec_result.success,
        &trace,
        &format!("script contract `{}` exec failed", script.id().name),
    )?;

    // 11. Print the script logs into the console.
    print_logs(&trace, &trace_context);

    // 12. Save the execution trace.
    let trace_file = trace_writer.write(&trace)?;
    info!(
        "execution trace for {} saved to {}",
        script.id(),
        trace_file.display()
    );

    Ok(())
}

/// Bail with a dumped execution trace when a call failed.
fn ensure_call_success(
    trace_writer: &ExecutionTraceWriter,
    success: bool,
    trace: &Trace,
    message: &str,
) -> Result<()> {
    if success {
        return Ok(());
    }
    let trace_file = trace_writer.write(trace)?;
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
