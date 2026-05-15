use clap::{Parser, Subcommand};

mod commands;

#[derive(Debug, Parser)]
#[command(name = "raptor", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Fuzz a Solidity smart contract.
    Fuzz(commands::fuzz::Args),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Fuzz(args) => commands::fuzz::run(args),
    }
}
