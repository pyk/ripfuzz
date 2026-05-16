//! CLI entry point for the Raptor fuzzer.

use anyhow::Result;
use clap::{Parser, Subcommand};
use clap_verbosity_flag::{InfoLevel, Verbosity};

use raptor::{commands, logger};

#[derive(Debug, Parser)]
#[command(name = "raptor", version, about)]
struct Cli {
    #[command(flatten)]
    verbosity: Verbosity<InfoLevel>,

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
    logger::init(cli.verbosity.tracing_level());

    match cli.command {
        Commands::Fuzz(args) => commands::fuzz::run(args),
    }
}
