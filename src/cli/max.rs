//! `max` CLI command implementation.

use anyhow::Result;
use clap::Parser;

/// Maximize a harness value.
#[derive(Debug, Parser)]
pub struct Args {
    /// Harness to maximize.
    #[arg(value_name = "HARNESS")]
    pub harness: String,
}

/// Run the `max` command.
pub fn run(args: Args) -> Result<()> {
    println!("{}", args.harness);
    Ok(())
}
