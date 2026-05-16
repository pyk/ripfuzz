//! `fuzz` CLI command implementation.

use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::Result;
use clap::Parser;
use tracing::{debug, info, instrument};

use crate::campaign::{Campaign, CampaignConfig};

static DEFAULT_WORKERS: LazyLock<String> = LazyLock::new(|| {
    let cores = libafl_bolts::core_affinity::get_core_ids()
        .map(|v| v.len())
        .unwrap_or(1);
    format!("{}", cores)
});

fn parse_workers(s: &str) -> Result<usize, String> {
    let n = s
        .parse::<usize>()
        .map_err(|e| format!("invalid worker count: {e}"))?;
    if n == 0 {
        return Err("workers must be at least 1".into());
    }
    Ok(n)
}

#[derive(Debug, Parser)]
pub struct Args {
    /// Path to the target contract (e.g. ./test/Contract.sol).
    #[arg(value_name = "TARGET_PATH")]
    pub target_path: PathBuf,

    /// Path to the Foundry project root.
    #[arg(long = "project", short = 'p')]
    pub project_path: Option<PathBuf>,

    /// Number of parallel workers to spawn.
    #[arg(short = 'w', long, default_value = DEFAULT_WORKERS.as_str(), value_parser = parse_workers)]
    pub workers: usize,

    /// Maximum number of campaign runs across all workers.
    #[arg(short = 'r', long = "max-runs", default_value = "10000")]
    pub max_runs: u64,

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

#[instrument(skip(args), fields(target = ?args.target_path, workers = args.workers, max_runs = args.max_runs))]
pub fn run(args: Args) -> Result<()> {
    let project_path = match args.project_path {
        Some(p) => {
            debug!(?p, "using explicit project path");
            p
        }
        None => {
            let cwd = env::current_dir()?;
            debug!(?cwd, "using current directory as project path");
            cwd
        }
    };
    let config = CampaignConfig {
        workers: args.workers,
        max_runs: args.max_runs,
        timeout_secs: args.timeout_secs,
        sequence_length: args.sequence_length,
        seed: args.seed,
        max_block_number_delay: args.max_block_number_delay,
        max_block_timestamp_delay: args.max_block_timestamp_delay,
        broker_port: 0,
    };
    info!(?config, "starting fuzzing campaign");

    let campaign = Campaign::for_target(&args.target_path)
        .with_project(&project_path)
        .with_config(config)
        .build()?;
    let artifact = campaign.artifact();

    println!("Loaded contract: {}", artifact.contract_name);
    println!(
        "Properties:      {:?}",
        artifact
            .properties
            .iter()
            .map(|(_, n)| n.as_str())
            .collect::<Vec<&str>>()
    );
    info!(contract = %artifact.contract_name, properties = artifact.properties.len(), "artifact loaded");

    let result = campaign.run()?;

    println!("Fuzzing completed: {} runs", result.runs);
    info!(
        runs = result.runs,
        failures = result.failures.len(),
        "campaign finished"
    );
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
            println!("{}", crate::worker::format_failure(artifact, failure));
        }
        let total = artifact.properties.len();
        let failed = result.failures.len();
        let passed = total.saturating_sub(failed);
        println!();
        println!("Test summary: {} passed, {} failed", passed, failed);
    }

    Ok(())
}
