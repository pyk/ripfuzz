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
    Run(cli::run::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run(args) => cli::run::run(args),
    };

    if let Err(e) = result {
        error!("{e:#}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
