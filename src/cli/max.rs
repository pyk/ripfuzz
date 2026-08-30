//! `max` CLI command implementation.

use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use clap::Parser;
use tracing::info;

use crate::cli::config::Config;
use crate::evm::{Chain, ChainConfig};
use crate::harness::HarnessId;
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

    /// Project root directory.
    #[arg(long, value_name = "PATH")]
    pub root: Option<PathBuf>,
}

/// Run the `max` command.
pub fn run(args: Args) -> Result<()> {
    // 1. Initialize tracing subscriber.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();

    // 2. Load configuration relative to the project root.
    let root = args.root.clone().unwrap_or_else(|| PathBuf::from("."));
    let config = Config::new().with_root(&root).load(&args.config)?;

    // 3. Compile harness via Solc relative to the project root.
    let harness = Solc::new()
        .with_version(&config.solc)
        .with_root(&root)
        .with_target(&args.harness.path)
        .with_name(&args.harness.name)
        .with_out(&config.out)
        .compile()?;

    // 4. Create the test chain the harness will be deployed to.
    let chain_config = ChainConfig::new(&root).coverage(true);
    let mut chain = Chain::new(chain_config)?;

    // 5. Deploy the harness contract.
    let deployment = chain.deploy(harness.deploy_input())?;
    ensure!(
        deployment.result.success,
        "harness contract `{}` deployment failed",
        harness.id.name
    );
    let address = deployment
        .address
        .context("deployment succeeded but created_address is missing")?;
    info!(harness = %harness.id, address = %address, "harness deployed");
    println!("{address}");
    Ok(())
}
