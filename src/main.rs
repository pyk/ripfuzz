//! CLI entry point for the Raptor fuzzer.

use anyhow::Result;
use clap::{Parser, Subcommand};
use raptor::commands;
use tracing_subscriber::EnvFilter;

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
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Fuzz(args) => commands::fuzz::run(args),
    }
}
