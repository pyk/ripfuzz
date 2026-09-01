//! CLI entry point for the Ripfuzz fuzzer.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tracing::error;

use ripfuzz::cli;

#[derive(Debug, Parser)]
#[command(name = "ripfuzz", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Initialize a new ripfuzz project.
    Init(cli::init::Args),
    /// Execute a script contract.
    Exec(Box<cli::exec::Args>),
    /// Find findings.
    Test(Box<cli::test::Args>),
    /// Find maximum value.
    Max(Box<cli::max::Args>),
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Load `.env` from the working directory or a parent so every command
    // (and the harness `vm.getEnv` cheatcode) sees the same environment.
    // Print directly because no logger is initialized yet.
    if let Err(e) = dotenvy::dotenv()
        && !e.not_found()
    {
        eprintln!("failed to load .env: {e}");
        return ExitCode::FAILURE;
    }

    let result = match cli.command {
        Commands::Init(args) => cli::init::run(args),
        Commands::Exec(args) => cli::exec::run(*args),
        Commands::Test(args) => cli::test::run(*args).map(|_| ()),
        Commands::Max(args) => cli::max::run(*args).map(|_| ()),
    };

    if let Err(e) = result {
        error!("{e:#}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
