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
    let target = TargetContractBuilder::build(&args.path, args.project.as_deref())?;

    let contract_name = args
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("<unknown>");

    println!("Loaded contract: {contract_name}");
    println!("Deployed at:     {}", target.deployed_address);
    println!(
        "Properties:      {:?}",
        target.properties.iter().map(|(_, n)| n).collect::<Vec<_>>()
    );

    let runner = EvmRunner::from_target(&target)?;
    let seeds = build_seeds(&target);
    crate::fuzzer::run(&runner, seeds)
}

fn build_seeds(target: &crate::target_contract::TargetContract) -> Vec<Vec<u8>> {
    let mut seeds = Vec::new();

    for func in target.abi.functions() {
        let selector = func.selector();
        let mut seed = selector.to_vec();
        seed.resize(36, 0);
        seeds.push(seed);
    }

    // Add a combined seed with all functions in order
    let mut combined = Vec::new();
    for func in target.abi.functions() {
        let selector = func.selector();
        combined.extend_from_slice(selector.as_slice());
        combined.resize(combined.len() + 32, 0);
    }
    if !combined.is_empty() {
        seeds.push(combined);
    }

    seeds
}
