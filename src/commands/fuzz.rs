use std::env;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use crate::contract::ContractBuilder;
use crate::fuzzer::Fuzzer;

#[derive(Debug, Parser)]
pub struct Args {
    /// Path to the target Solidity file (e.g. ./test/Contract.sol).
    pub path: PathBuf,

    /// Path to the Foundry project root.
    #[arg(long, short = 'p')]
    pub project: Option<PathBuf>,
}

pub fn run(args: Args) -> Result<()> {
    let project_path = args.project.unwrap_or_else(|| env::current_dir().unwrap());
    let artifact = ContractBuilder::build(&project_path, &args.path)?;

    println!("Loaded contract: {}", artifact.contract_name);
    println!(
        "Properties:      {:?}",
        artifact
            .properties
            .iter()
            .map(|(_, n)| n)
            .collect::<Vec<_>>()
    );

    let fuzzer = Fuzzer::from_artifact(artifact)?;
    let result = fuzzer.run()?;

    println!("Fuzzing completed: {} iterations", result.iterations);
    if result.crashes.is_empty() {
        println!("No crashes found.");
    } else {
        println!("Found {} crash(es)", result.crashes.len());
    }

    Ok(())
}
