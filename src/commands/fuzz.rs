//! `fuzz` CLI command implementation.

use std::env;
use std::path::PathBuf;

use alloy_primitives::Address;
use anyhow::Result;
use clap::Parser;
use revm::primitives::U256;
use tracing::{debug, info, instrument};

use crate::campaign::{Campaign, CampaignConfig};
use crate::contract::resolve_coverage_to_source;

fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

fn parse_threads(s: &str) -> Result<usize, String> {
    let n = s
        .parse::<usize>()
        .map_err(|e| format!("invalid thread count: {e}"))?;
    if n == 0 {
        return Err("threads must be at least 1".into());
    }
    Ok(n)
}

fn parse_balance(s: &str) -> Result<U256, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(U256::ZERO);
    }

    let lower = trimmed.to_lowercase();
    if let Some(stripped) = lower.strip_prefix("0x") {
        return U256::from_str_radix(stripped, 16).map_err(|e| format!("invalid hex balance: {e}"));
    }

    if trimmed.contains(['e', 'E']) {
        let f = trimmed
            .parse::<f64>()
            .map_err(|e| format!("invalid scientific notation balance: {e}"))?;
        let plain = format!("{:.0}", f);
        return U256::from_str_radix(&plain, 10)
            .map_err(|e| format!("invalid scientific notation balance: {e}"));
    }

    U256::from_str_radix(trimmed, 10).map_err(|e| format!("invalid decimal balance: {e}"))
}

fn parse_address(s: &str) -> Result<Address, String> {
    let trimmed = s.trim();
    let mut hex = String::from(trimmed.trim_start_matches("0x").trim_start_matches("0X"));
    if !hex.len().is_multiple_of(2) {
        hex.insert(0, '0');
    }
    let bytes = hex::decode(&hex).map_err(|e| format!("invalid hex address: {e}"))?;
    if bytes.len() > 20 {
        return Err("address exceeds 20 bytes".into());
    }
    let mut padded = [0u8; 20];
    padded[20 - bytes.len()..].copy_from_slice(&bytes);
    Ok(Address::new(padded))
}

#[derive(Debug, Parser)]
pub struct Args {
    /// Path to the target contract (e.g. ./test/Contract.sol).
    #[arg(value_name = "TARGET_PATH")]
    pub target_path: PathBuf,

    // Project & Deployment
    /// Path to the Foundry project root.
    #[arg(
        short = 'p',
        long = "project",
        value_name = "PATH",
        help_heading = "Project & Deployment"
    )]
    pub project_path: Option<PathBuf>,

    /// Wei to send during target contract deployment.
    #[arg(long = "deploy-value", default_value = "0", value_parser = parse_balance, value_name = "WEI", help_heading = "Project & Deployment")]
    pub deploy_value: U256,

    /// Account address used to deploy the target contract.
    #[arg(
        long = "deployer",
        default_value_t = crate::chain::init::DEFAULT_DEPLOYER,
        value_parser = parse_address,
        value_name = "ADDRESS",
        help_heading = "Project & Deployment"
    )]
    pub deployer_address: Address,

    // Campaign Limits
    /// Number of parallel fuzzer threads to spawn.
    #[arg(short = 'w', long = "threads", default_value_t = default_threads(), value_parser = parse_threads, value_name = "N", help_heading = "Campaign Limits")]
    pub threads: usize,

    /// Maximum number of campaign runs across all fuzzers.
    #[arg(
        short = 'r',
        long = "max-runs",
        default_value = "10000",
        value_name = "N",
        help_heading = "Campaign Limits"
    )]
    pub max_runs: u64,

    /// Timeout in seconds for the entire fuzzing campaign.
    #[arg(
        short = 't',
        long = "timeout",
        value_name = "SECS",
        help_heading = "Campaign Limits"
    )]
    pub timeout_secs: Option<u64>,

    // Sequence
    /// Maximum number of calls in each generated fuzzing sequence.
    #[arg(
        short = 'c',
        long = "max-calls",
        default_value = "32",
        value_name = "N",
        help_heading = "Fuzzing Parameters"
    )]
    pub sequence_length: usize,

    /// Random seed for reproducibility.
    #[arg(
        long = "seed",
        default_value = "0",
        value_name = "N",
        help_heading = "Fuzzing Parameters"
    )]
    pub seed: u64,

    /// Maximum block number delay between calls.
    #[arg(
        long = "max-block-delay",
        default_value = "5",
        value_name = "N",
        help_heading = "Fuzzing Parameters"
    )]
    pub max_block_number_delay: u64,

    /// Maximum block timestamp delay between calls.
    #[arg(
        long = "max-time-delay",
        default_value = "5",
        value_name = "N",
        help_heading = "Fuzzing Parameters"
    )]
    pub max_block_timestamp_delay: u64,

    // Corpus
    /// Directory to load and persist coverage-guided corpus files.
    #[arg(long = "corpus-dir", value_name = "DIR", help_heading = "Corpus")]
    pub corpus_dir: Option<PathBuf>,

    // Logging
    /// Increase logging verbosity.
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count, help_heading = "Logging")]
    pub verbose: u8,

    /// Decrease logging verbosity.
    #[arg(short = 'q', long = "quiet", action = clap::ArgAction::Count, help_heading = "Logging")]
    pub quiet: u8,

    // Security
    /// Enable the `ffi` cheatcode (security-sensitive).
    #[arg(long = "ffi", help_heading = "Security")]
    pub ffi: bool,
}

impl Args {
    pub fn tracing_level(&self) -> tracing::Level {
        match self.verbose as i32 - self.quiet as i32 {
            i32::MIN..=-2 => tracing::Level::ERROR,
            -1 => tracing::Level::WARN,
            0 => tracing::Level::INFO,
            1 => tracing::Level::DEBUG,
            2..=i32::MAX => tracing::Level::TRACE,
        }
    }
}

#[instrument(skip(args), fields(target = ?args.target_path, threads = args.threads, max_runs = args.max_runs))]
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
        threads: args.threads,
        max_runs: args.max_runs,
        timeout_secs: args.timeout_secs,
        sequence_length: args.sequence_length,
        seed: args.seed,
        max_block_number_delay: args.max_block_number_delay,
        max_block_timestamp_delay: args.max_block_timestamp_delay,
        corpus_dir: args.corpus_dir,
        ffi: args.ffi,
        deploy_value: args.deploy_value,
        deployer_address: args.deployer_address,
    };
    info!(?config, "starting fuzzing campaign");

    let campaign = Campaign::for_target(&args.target_path)
        .with_project(&project_path)
        .with_config(config)
        .build()?;
    let artifact = campaign.artifact();

    info!(target: "raptor::user", "Loaded contract: {}", artifact.contract_name);
    let invariant_names: Vec<&str> = artifact
        .invariants
        .iter()
        .map(|(_, n)| n.as_str())
        .collect();
    info!(target: "raptor::user", "Invariants:      {:?}", invariant_names);
    info!(contract = %artifact.contract_name, invariants = artifact.invariants.len(), "artifact loaded");

    let result = campaign.run()?;

    let elapsed_secs = result.elapsed_secs;
    let calls_per_sec = if elapsed_secs > 0.0 {
        result.total_calls as f64 / elapsed_secs
    } else {
        0.0
    };
    let avg_gas_per_call = if result.total_calls > 0 {
        result.total_gas as f64 / result.total_calls as f64
    } else {
        0.0
    };

    info!(target: "raptor::user", "Fuzzing completed: {} runs, {} calls", result.runs, result.total_calls);
    info!(target: "raptor::user", "Throughput: {:.0} calls/sec", calls_per_sec);
    info!(target: "raptor::user", "Average gas per call: {:.0}", avg_gas_per_call);
    info!(
        runs = result.runs,
        total_calls = result.total_calls,
        total_gas = result.total_gas,
        failures = result.failures.len(),
        "campaign finished"
    );
    if result.failures.is_empty() {
        info!(target: "raptor::user", "All invariants passed.");
    } else {
        for failure in &result.failures {
            info!(target: "raptor::user", "");
            info!(target: "raptor::user", "[FAILED] Invariant Test: {}::{}", artifact.contract_name, failure.function_name);
            info!(target: "raptor::user", "Test for method \"{}::{}\" failed after the following call sequence:", artifact.contract_name, failure.function_name);
            info!(target: "raptor::user", "[Call Sequence]");
            info!(target: "raptor::user", "{}", crate::fuzzer::format_failure(artifact, failure, result.deployer_address));
        }
        let total = artifact.invariants.len();
        let failed = result.failures.len();
        let passed = total.saturating_sub(failed);
        info!(target: "raptor::user", "");
        info!(target: "raptor::user", "Test summary: {} passed, {} failed", passed, failed);
    }

    // Source-level coverage summary
    let report = resolve_coverage_to_source(&result.coverage, artifact);
    if report.hit_count() > 0 {
        info!(target: "raptor::user", "Coverage: {} unique source locations hit", report.hit_count());
    } else {
        info!(target: "raptor::user", "Coverage: no source locations hit (source map may be missing)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_address, parse_balance};
    use alloy_primitives::Address;
    use revm::primitives::U256;

    #[test]
    fn parse_balance_empty() {
        assert_eq!(parse_balance("").unwrap(), U256::ZERO);
    }

    #[test]
    fn parse_balance_zero() {
        assert_eq!(parse_balance("0").unwrap(), U256::ZERO);
    }

    #[test]
    fn parse_balance_decimal() {
        assert_eq!(parse_balance("1000").unwrap(), U256::from(1000));
    }

    #[test]
    fn parse_balance_hex() {
        assert_eq!(parse_balance("0x1a2b").unwrap(), U256::from(6699));
    }

    #[test]
    fn parse_balance_hex_uppercase() {
        assert_eq!(parse_balance("0x1A2B").unwrap(), U256::from(6699));
    }

    #[test]
    fn parse_balance_scientific_lower() {
        assert_eq!(
            parse_balance("1e18").unwrap(),
            U256::from(1_000_000_000_000_000_000u128)
        );
    }

    #[test]
    fn parse_balance_scientific_upper() {
        assert_eq!(
            parse_balance("1E18").unwrap(),
            U256::from(1_000_000_000_000_000_000u128)
        );
    }

    #[test]
    fn parse_balance_invalid_hex() {
        assert!(parse_balance("0xzz").is_err());
    }

    #[test]
    fn parse_balance_invalid_decimal() {
        assert!(parse_balance("abc").is_err());
    }

    #[test]
    fn parse_balance_scientific_rounds() {
        // 1.5e18 parses as f64 and rounds to 1500000000000000000,
        // which is valid under Medusa's rules.
        assert_eq!(
            parse_balance("1.5e18").unwrap(),
            U256::from(1_500_000_000_000_000_000u128)
        );
    }

    #[test]
    fn parse_address_full() {
        assert_eq!(
            parse_address("0xc34296175b9e78f66edbeaeb7acea4c615c092e1").unwrap(),
            Address::new([
                0xc3, 0x42, 0x96, 0x17, 0x5b, 0x9e, 0x78, 0xf6, 0x6e, 0xdb, 0xea, 0xeb, 0x7a, 0xce,
                0xa4, 0xc6, 0x15, 0xc0, 0x92, 0xe1,
            ])
        );
    }

    #[test]
    fn parse_address_no_prefix() {
        assert_eq!(
            parse_address("c34296175b9e78f66edbeaeb7acea4c615c092e1").unwrap(),
            Address::new([
                0xc3, 0x42, 0x96, 0x17, 0x5b, 0x9e, 0x78, 0xf6, 0x6e, 0xdb, 0xea, 0xeb, 0x7a, 0xce,
                0xa4, 0xc6, 0x15, 0xc0, 0x92, 0xe1,
            ])
        );
    }

    #[test]
    fn parse_address_short_odd() {
        assert_eq!(
            parse_address("0x30000").unwrap(),
            Address::new([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x03, 0x00, 0x00,
            ])
        );
    }

    #[test]
    fn parse_address_short_even() {
        assert_eq!(
            parse_address("0x10000").unwrap(),
            Address::new([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            ])
        );
    }

    #[test]
    fn parse_address_four_hex() {
        assert_eq!(
            parse_address("0xabcd").unwrap(),
            Address::new([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0xab, 0xcd,
            ])
        );
    }

    #[test]
    fn parse_address_zero() {
        assert_eq!(parse_address("0x0").unwrap(), Address::ZERO);
    }

    #[test]
    fn parse_address_empty_after_prefix() {
        assert_eq!(parse_address("0x").unwrap(), Address::ZERO);
    }

    #[test]
    fn parse_address_all_zeros() {
        assert_eq!(
            parse_address("0000000000000000000000000000000000000000").unwrap(),
            Address::ZERO
        );
    }

    #[test]
    fn parse_address_invalid_hex() {
        assert!(parse_address("0xzz").is_err());
    }

    #[test]
    fn parse_address_too_long() {
        assert!(parse_address("0x000000000000000000000000000000000000000000").is_err());
    }
}
