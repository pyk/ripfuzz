use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
pub struct Args {
    /// Path to the target Solidity file.
    pub path: PathBuf,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    println!("Fuzzing target: {}", args.path.display());
    Ok(())
}
