//! `fuzz` CLI command implementation.

use std::env;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use crate::contract;
use crate::fuzzer;

#[derive(Debug, Parser)]
pub struct Args {
    /// Path to the target contract (e.g. ./test/Contract.sol).
    pub path: PathBuf,

    /// Path to the Foundry project root.
    #[arg(long, short = 'p')]
    pub project: Option<PathBuf>,

    /// Cores to use for parallel fuzzing (e.g. "all", "1,2,3", "0-3").
    #[arg(long)]
    pub cores: Option<String>,
}

pub fn run(args: Args) -> Result<()> {
    let project_path = match args.project {
        Some(p) => p,
        None => env::current_dir()?,
    };
    let artifact = contract::ContractBuilder::build(&project_path, &args.path)?;

    println!("Loaded contract: {}", artifact.contract_name);
    println!(
        "Properties:      {:?}",
        artifact
            .properties
            .iter()
            .map(|(_, n)| n)
            .collect::<Vec<_>>()
    );

    let contract_name = artifact.contract_name.clone();
    let properties = artifact.properties.clone();
    let fuzzer = fuzzer::Fuzzer::from_artifact(artifact)?;

    if let Some(cores_str) = args.cores {
        let cores = libafl_bolts::core_affinity::Cores::from_cmdline(&cores_str)?;
        println!("Launching parallel fuzzer on cores: {}", cores_str);
        fuzzer.launch(&cores)?;
        return Ok(());
    }

    let result = fuzzer.run()?;

    println!("Fuzzing completed: {} iterations", result.iterations);
    if result.failures.is_empty() {
        println!("All properties passed.");
    } else {
        for failure in &result.failures {
            println!();
            println!(
                "[FAILED] Property Test: {}::{}",
                contract_name, failure.property_name
            );
            println!(
                "Test for method \"{}::{}\" failed after the following call sequence:",
                contract_name, failure.property_name
            );
            println!("[Call Sequence]");
            println!("{}", fuzzer.format_failure(failure));
        }
        let total = properties.len();
        let failed = result.failures.len();
        let passed = total.saturating_sub(failed);
        println!();
        println!("Test summary: {} passed, {} failed", passed, failed);
    }

    Ok(())
}
