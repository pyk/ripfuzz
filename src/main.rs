//! CLI entry point for the Raptor fuzzer.

use anyhow::Result;
use clap::{Parser, Subcommand};

use raptor::{commands, logger};

#[derive(Debug, Parser)]
#[command(name = "raptor", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start a fuzzing campaign.
    Fuzz(commands::fuzz::Args),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Fuzz(args) => {
            logger::init(Some(args.tracing_level()));
            commands::fuzz::run(args)
        }
    }
}
