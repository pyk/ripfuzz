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
    Init(cli::init::Command),
    /// Execute a script contract.
    Exec(Box<cli::exec::Command>),
    /// Find broken invariants.
    Test(Box<cli::test::Command>),
    /// Find maximum value.
    Max(Box<cli::max::Command>),
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
        Commands::Init(command) => command.run(),
        Commands::Exec(command) => command.run(),
        Commands::Test(command) => command.run().map(|_| ()),
        Commands::Max(command) => command.run().map(|_| ()),
    };

    if let Err(e) = result {
        error!("{e:#}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
