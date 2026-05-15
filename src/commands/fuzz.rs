use std::path::{Path, PathBuf};

use clap::Parser;
use revm::primitives::keccak256;

use crate::evm::EvmRunner;
use crate::foundry::FoundryArtifact;

#[derive(Debug, Parser)]
pub struct Args {
    /// Path to the target Solidity file.
    pub path: PathBuf,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    let artifact_path = find_artifact(&args.path)?;
    println!("Loading artifact: {}", artifact_path.display());

    let artifact = FoundryArtifact::from_file(&artifact_path)?;
    let runner = EvmRunner::deploy(&artifact)?;
    println!("Deployed contract at: {}", runner.contract_address);

    let seeds = build_seeds(&artifact);
    crate::fuzzer::run(&runner, seeds)
}

fn find_artifact(sol_path: &Path) -> anyhow::Result<PathBuf> {
    let file_name = sol_path
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid source path"))?;

    let contract_name = file_name
        .strip_suffix(".sol")
        .ok_or_else(|| anyhow::anyhow!("path must end with .sol"))?;

    let candidate = PathBuf::from("out")
        .join(file_name)
        .join(format!("{}.json", contract_name));

    if candidate.exists() {
        return Ok(candidate);
    }

    anyhow::bail!(
        "could not find artifact at {}. Make sure you ran `forge build` first.",
        candidate.display()
    )
}

fn build_seeds(artifact: &FoundryArtifact) -> Vec<Vec<u8>> {
    let mut seeds = Vec::new();

    for sig in artifact.method_identifiers.keys() {
        let selector = &keccak256(sig.as_bytes())[..4];
        let mut seed = selector.to_vec();
        seed.resize(36, 0);
        seeds.push(seed);
    }

    // Add a combined seed with all functions in order
    let mut combined = Vec::new();
    for sig in artifact.method_identifiers.keys() {
        let selector = &keccak256(sig.as_bytes())[..4];
        combined.extend_from_slice(selector);
        combined.resize(combined.len() + 32, 0);
    }
    if !combined.is_empty() {
        seeds.push(combined);
    }

    seeds
}
