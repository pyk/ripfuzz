use std::env;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use crate::evm::EvmRunner;
use crate::target_contract::TargetContractBuilder;

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
    let artifact = TargetContractBuilder::build(&project_path, &args.path)?;

    println!("Loaded contract: {}", artifact.contract_name);
    println!(
        "Properties:      {:?}",
        artifact.properties.iter().map(|(_, n)| n).collect::<Vec<_>>()
    );

    let runner = EvmRunner::from_target(&artifact)?;
    let seeds = build_seeds(&artifact);
    crate::fuzzer::run(&runner, seeds)
}

fn build_seeds(artifact: &crate::target_contract::TargetContractArtifact) -> Vec<Vec<u8>> {
    let mut seeds = Vec::new();

    for func in artifact.abi.functions() {
        let selector = func.selector();
        let mut seed = selector.to_vec();
        seed.resize(36, 0);
        seeds.push(seed);
    }

    // Add a combined seed with all functions in order
    let mut combined = Vec::new();
    for func in artifact.abi.functions() {
        let selector = func.selector();
        combined.extend_from_slice(selector.as_slice());
        combined.resize(combined.len() + 32, 0);
    }
    if !combined.is_empty() {
        seeds.push(combined);
    }

    seeds
}
