//! `max` CLI command implementation.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use crate::cli::config::Config;
use crate::cli::harness_id::HarnessId;

/// Maximize a harness value.
#[derive(Debug, Parser)]
pub struct Args {
    /// Harness to maximize.
    #[arg(value_name = "HARNESS")]
    pub harness: HarnessId,

    /// Path to the ripfuzz config file.
    #[arg(long, default_value = "ripfuzz.toml", value_name = "PATH")]
    pub config: PathBuf,
}

/// Run the `max` command.
pub fn run(args: Args) -> Result<()> {
    let config = Config::load(&args.config)?;
    println!("{}", config.solc);
    Ok(())
}
