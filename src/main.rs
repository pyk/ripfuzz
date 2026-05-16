//! CLI entry point for the Raptor fuzzer.

use anyhow::Result;
use clap::{Parser, Subcommand};
use raptor::commands;

#[derive(Debug, Parser)]
#[command(name = "raptor", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start a property-based fuzzing campaign.
    Fuzz(commands::fuzz::Args),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Fuzz(args) => commands::fuzz::run(args),
    }
}
