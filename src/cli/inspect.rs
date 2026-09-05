//! `inspect` CLI command implementation.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::harness::HarnessId;
use crate::inspectors::{ExternalFunctionsInspector, FunctionSourceInspector};
use crate::logger::Logger;

/// Inspect compiled contracts.
#[derive(Debug, Parser)]
pub struct Command {
    #[command(subcommand)]
    pub command: Commands,
}

/// Subcommands of `ripfuzz inspect`.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Print the external functions of a contract.
    ExternalFunctions(ExternalFunctions),

    /// Print the source of a function resolved by selector.
    FunctionSource(FunctionSource),
}

/// Print the external functions of a contract.
#[derive(Debug, Parser)]
pub struct ExternalFunctions {
    /// Contract file to inspect, e.g. `src/Voter.sol` or `src/Voter.sol:Voter`.
    #[arg(value_name = "CONTRACT")]
    pub contract: HarnessId,

    /// Path to the ripfuzz config file.
    #[arg(long, default_value = "ripfuzz.toml", value_name = "PATH")]
    pub config: PathBuf,

    /// Project root directory.
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub root: PathBuf,

    /// Suppress terminal log output.
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Log verbosity level.
    #[arg(long, default_value = "info", value_name = "LEVEL")]
    pub log_level: tracing::Level,
}

impl Command {
    /// Run the `inspect` command.
    pub fn run(&self) -> Result<()> {
        match &self.command {
            Commands::ExternalFunctions(command) => command.run(),
            Commands::FunctionSource(command) => command.run(),
        }
    }
}

impl ExternalFunctions {
    /// Run the `inspect external-functions` command.
    pub fn run(&self) -> Result<()> {
        // 1. Initialize logging.
        Logger::new()
            .with_root(&self.root)
            .with_quiet(self.quiet)
            .with_level(self.log_level)
            .init()?;

        // 2. Load configuration relative to the project root.
        let config = Config::new().with_root(&self.root).load(&self.config)?;

        // 3. Inspect the contract and print the report.
        let output = ExternalFunctionsInspector::new(&self.root, config).inspect(&self.contract)?;
        println!("{output}");

        Ok(())
    }
}

/// Print the source of a function resolved by selector.
#[derive(Debug, Parser)]
pub struct FunctionSource {
    /// Contract file to inspect, e.g. `src/Voter.sol` or `src/Voter.sol:Voter`.
    #[arg(value_name = "CONTRACT")]
    pub contract: HarnessId,

    /// 4-byte function selector, e.g. `f02e634e`.
    #[arg(value_name = "SELECTOR")]
    pub selector: String,

    /// Path to the ripfuzz config file.
    #[arg(long, default_value = "ripfuzz.toml", value_name = "PATH")]
    pub config: PathBuf,

    /// Project root directory.
    #[arg(long, default_value = ".", value_name = "PATH")]
    pub root: PathBuf,

    /// Suppress terminal log output.
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Log verbosity level.
    #[arg(long, default_value = "info", value_name = "LEVEL")]
    pub log_level: tracing::Level,
}

impl FunctionSource {
    /// Run the `inspect function-source` command.
    pub fn run(&self) -> Result<()> {
        // 1. Initialize logging.
        Logger::new()
            .with_root(&self.root)
            .with_quiet(self.quiet)
            .with_level(self.log_level)
            .init()?;

        // 2. Load configuration relative to the project root.
        let config = Config::new().with_root(&self.root).load(&self.config)?;

        // 3. Inspect the function and print the report.
        let output = FunctionSourceInspector::new(&self.root, config)
            .inspect(&self.contract, &self.selector)?;
        println!("{output}");

        Ok(())
    }
}
