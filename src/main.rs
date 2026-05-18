//! CLI entry point for the Raptor fuzzer.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tracing::error;

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

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Fuzz(args) => {
            logger::init(Some(args.tracing_level()));
            commands::fuzz::run(args)
        }
    };

    if let Err(e) = result {
        error!(target: "raptor::user", "{e}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
