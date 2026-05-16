//! `fuzz` CLI command implementation.

use std::env;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use crate::campaign::{Campaign, CampaignConfig};

#[derive(Debug, Parser)]
pub struct Args {
    /// Path to the target contract (e.g. ./test/Contract.sol).
    #[arg(value_name = "TARGET_PATH")]
    pub target_path: PathBuf,

    /// Path to the Foundry project root.
    #[arg(long = "project", short = 'p')]
    pub project_path: Option<PathBuf>,

    /// Number of parallel workers to spawn (0 = use all available cores).
    #[arg(long, default_value = "0")]
    pub workers: usize,

    /// Maximum number of fuzzing iterations.
    #[arg(long = "fuzz-iters", default_value = "10000")]
    pub max_iters: u64,

    /// Timeout in seconds for the entire fuzzing campaign.
    #[arg(long = "fuzz-timeout", default_value = "60")]
    pub timeout_secs: u64,

    /// Maximum number of calls in a generated sequence.
    #[arg(long = "fuzz-seq-len", default_value = "5")]
    pub sequence_length: usize,

    /// Random seed for reproducibility.
    #[arg(long = "fuzz-seed", default_value = "0")]
    pub seed: u64,

    /// Maximum block number delay between calls.
    #[arg(long = "max-block-delay", default_value = "5")]
    pub max_block_number_delay: u64,

    /// Maximum block timestamp delay between calls.
    #[arg(long = "max-time-delay", default_value = "5")]
    pub max_block_timestamp_delay: u64,
}

pub fn run(args: Args) -> Result<()> {
    let project = match args.project_path {
        Some(p) => p,
        None => env::current_dir()?,
    };
    let config = CampaignConfig {
        workers: args.workers,
        max_iters: args.max_iters,
        timeout_secs: args.timeout_secs,
        sequence_length: args.sequence_length,
        seed: args.seed,
        max_block_number_delay: args.max_block_number_delay,
        max_block_timestamp_delay: args.max_block_timestamp_delay,
    };
    let campaign = Campaign::for_target(&args.target_path, &project)
        .with_config(config)
        .build()?;
    let artifact = campaign.artifact();

    println!("Loaded contract: {}", artifact.contract_name);
    println!(
        "Properties:      {:?}",
        artifact
            .properties
            .iter()
            .map(|(_, n)| n)
            .collect::<Vec<_>>()
    );

    let result = campaign.run()?;

    println!("Fuzzing completed: {} iterations", result.iterations);
    if result.failures.is_empty() {
        println!("All properties passed.");
    } else {
        for failure in &result.failures {
            println!();
            println!(
                "[FAILED] Property Test: {}::{}",
                artifact.contract_name, failure.property_name
            );
            println!(
                "Test for method \"{}::{}\" failed after the following call sequence:",
                artifact.contract_name, failure.property_name
            );
            println!("[Call Sequence]");
            println!("{}", crate::fuzzer::format_failure(artifact, failure));
        }
        let total = artifact.properties.len();
        let failed = result.failures.len();
        let passed = total.saturating_sub(failed);
        println!();
        println!("Test summary: {} passed, {} failed", passed, failed);
    }

    Ok(())
}
