//! CLI entry point for the Ripfuzz fuzzer.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tracing::error;

use ripfuzz::commands;

#[derive(Debug, Parser)]
#[command(name = "ripfuzz", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start a fuzzing campaign.
    Fuzz(commands::fuzz::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Fuzz(args) => commands::fuzz::run(args),
    };

    if let Err(e) = result {
        error!("{e:#}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
