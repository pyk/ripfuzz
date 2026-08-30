//! `max` CLI command implementation.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use tracing::info;

use crate::cli::config::Config;
use crate::cli::harness_id::HarnessId;
use crate::solc::Solc;

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
    // 1. Initialize tracing subscriber.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();

    // 2. Load configuration.
    let config = Config::load(&args.config)?;

    // 3. Compile harness via Solc.
    Solc::new()
        .with_version(&config.solc)
        .with_target(&args.harness.path)
        .with_out(&config.out)
        .compile()?;

    // 4. Log and print solc version.
    info!(solc = %config.solc, "solc ready");
    println!("{}", config.solc);
    Ok(())
}
