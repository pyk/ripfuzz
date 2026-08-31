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
    /// Run a fuzzing campaign.
    Run(Box<cli::run::Args>),
    /// Maximize a harness value.
    Max(Box<cli::max::Args>),
    /// Initialize a new ripfuzz project.
    Init(cli::init::Args),
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
        Commands::Run(args) => cli::run::run(*args),
        Commands::Max(args) => cli::max::run(*args).map(|_| ()),
        Commands::Init(args) => cli::init::run(args),
    };

    if let Err(e) = result {
        error!("{e:#}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
