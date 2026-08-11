//! `run` CLI command implementation.

use std::path::PathBuf;

use alloy_primitives::Address;
use anyhow::Result;
use clap::Parser;
use revm::primitives::U256;
use tracing::instrument;

use crate::campaigns::{CampaignKind, CampaignSession, InvariantCampaign, MaxxingCampaign};

#[derive(Debug, Parser)]
pub struct Args {
    /// Harness contract identifier: bare name (`Harness`) or full artifact id
    /// (`src/Harness.sol:Harness`).
    #[arg(value_name = "HARNESS")]
    pub harness: String,

    // Project & Deployment
    /// Path to the Foundry project root.
    #[arg(
        short = 'p',
        long = "project",
        value_name = "PATH",
        help_heading = "Project & Deployment"
    )]
    pub project_path: Option<PathBuf>,

    /// Wei to send during harness contract deployment.
    #[arg(long = "deploy-value", default_value = "0", value_parser = Args::parse_balance, value_name = "WEI", help_heading = "Project & Deployment")]
    pub deploy_value: U256,

    /// Account address used to deploy the harness contract.
    #[arg(
        long = "deployer",
        default_value_t = crate::evm::DEFAULT_DEPLOYER,
        value_parser = Args::parse_address,
        value_name = "ADDRESS",
        help_heading = "Project & Deployment"
    )]
    pub deployer_address: Address,

    // Campaign Limits
    /// Gas limit for each fuzzer-generated transaction.
    #[arg(
        long = "gas-limit",
        default_value = "12500000",
        value_name = "GAS",
        help_heading = "Campaign Limits"
    )]
    pub gas_limit: u64,

    /// Number of parallel fuzzer threads to spawn.
    #[arg(short = 'w', long = "threads", default_value_t = Args::default_threads(), value_parser = Args::parse_threads, value_name = "N", help_heading = "Campaign Limits")]
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

    /// Maximum number of distinct failed assertions to collect before stopping
    /// the fuzzing campaign.
    #[arg(
        long = "max-failures",
        default_value = "1",
        value_parser = Args::parse_max_failures,
        value_name = "N",
        help_heading = "Campaign Limits"
    )]
    pub max_failures: usize,

    /// Timeout in seconds for the entire fuzzing campaign.
    #[arg(
        short = 't',
        long = "timeout",
        value_name = "SECS",
        help_heading = "Campaign Limits"
    )]
    pub timeout_secs: Option<u64>,

    /// Maximum number of calls in each generated fuzzing sequence.
    #[arg(
        short = 'c',
        long = "max-calls",
        default_value = "100",
        value_name = "N",
        help_heading = "Fuzzing Parameters"
    )]
    pub max_calls: usize,

    /// Random seed for reproducibility.
    ///
    /// When not provided, a random seed is generated and printed at campaign
    /// start so the run can be reproduced later.
    #[arg(long = "seed", value_name = "N", help_heading = "Fuzzing Parameters")]
    pub seed: Option<u64>,

    // Shrinker
    /// Maximum number of shrink runs across all shrinker threads.
    #[arg(
        long = "shrink-runs",
        default_value = "10000",
        value_name = "N",
        help_heading = "Shrinker"
    )]
    pub shrink_runs: u64,

    /// Timeout in seconds for the shrinking phase.
    #[arg(
        long = "shrink-timeout",
        value_name = "SECS",
        help_heading = "Shrinker"
    )]
    pub shrink_timeout_secs: Option<u64>,

    /// Number of parallel shrinker threads to spawn.
    #[arg(
        long = "shrink-threads",
        value_parser = Args::parse_threads,
        value_name = "N",
        help_heading = "Shrinker"
    )]
    pub shrink_threads: Option<usize>,

    // Corpus
    /// Directory to load and persist coverage-guided corpus files.
    #[arg(long = "corpus-dir", value_name = "DIR", help_heading = "Corpus")]
    pub corpus_dir: Option<PathBuf>,

    // Logging
    /// Log verbosity level.
    #[arg(
        long = "log-level",
        default_value = "info",
        value_name = "LEVEL",
        help_heading = "Logging"
    )]
    pub log_level: tracing::Level,

    /// Disable all log output (terminal and campaign log file).
    #[arg(long = "disable-log", help_heading = "Logging")]
    pub disable_log: bool,

    // Security
    /// Enable the `ffi` cheatcode (security-sensitive).
    #[arg(long = "ffi", help_heading = "Security")]
    pub ffi: bool,

    // Foundry
    /// Skip cache and force recompilation.
    #[arg(long = "force", help_heading = "Foundry")]
    pub force: bool,

    /// Treat any transaction revert as a failed assertion.
    #[arg(long = "fail-on-revert", help_heading = "Fuzzing Parameters")]
    pub fail_on_revert: bool,

    /// Additional Foundry projects whose build artifacts are loaded for
    /// coverage and trace resolution.
    ///
    /// Useful in fork mode when the harness contract interacts with
    /// contracts compiled in separate projects. Each path must point to a
    /// Foundry project root that contains an `out/` directory with compiled
    /// artifacts (run `forge build --ast --extra-output storageLayout` there
    /// first).
    ///
    /// Artifacts from these projects are merged into the coverage reporter so
    /// that on-chain bytecodes executed during fork mode can be matched back
    /// to their source maps and source files.
    #[arg(
        long = "external-project",
        value_name = "PATH",
        help_heading = "Project & Deployment"
    )]
    pub external_projects: Vec<PathBuf>,
}

impl Args {
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

    fn parse_max_failures(s: &str) -> Result<usize, String> {
        let n = s
            .parse::<usize>()
            .map_err(|e| format!("invalid max-failures value: {e}"))?;
        if n == 0 {
            return Err("max-failures must be at least 1".into());
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
            return U256::from_str_radix(stripped, 16)
                .map_err(|e| format!("invalid hex balance: {e}"));
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
}

/// Run a fuzzing campaign.
#[instrument(skip(args), fields(harness = ?args.harness, threads = args.threads, max_runs = args.max_runs))]
pub fn run(args: Args) -> Result<()> {
    let session = CampaignSession::new(args)?;
    match session.kind {
        CampaignKind::Invariant => InvariantCampaign::new(session)?.run(),
        CampaignKind::Maxxing => MaxxingCampaign::new(session)?.run(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use revm::primitives::U256;

    use crate::evm::DEFAULT_DEPLOYER;

    use super::*;

    fn count_corpus_files(dir: impl AsRef<Path>) -> usize {
        let dir = dir.as_ref();
        if !dir.exists() {
            return 0;
        }
        walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension() == Some("json".as_ref()))
            .count()
    }

    fn make_args(corpus_dir: impl AsRef<Path>) -> Args {
        let corpus_dir = corpus_dir.as_ref().to_path_buf();
        Args {
            harness: "src/L1SimpleKnob.sol:SimpleKnob".to_owned(),
            project_path: Some(PathBuf::from("fixtures/challenges")),
            deploy_value: U256::ZERO,
            deployer_address: DEFAULT_DEPLOYER,
            threads: 1,
            max_runs: 10000,
            max_failures: 1,
            timeout_secs: None,
            gas_limit: 12_500_000,
            max_calls: 32,
            seed: Some(0),
            corpus_dir: Some(corpus_dir),
            log_level: tracing::Level::INFO,
            disable_log: true,
            ffi: false,
            force: false,
            fail_on_revert: false,
            external_projects: Vec::new(),
            shrink_runs: 1,
            shrink_timeout_secs: None,
            shrink_threads: None,
        }
    }

    /// Regression test: once a bug is found, the corpus must not grow on
    /// subsequent runs.
    #[test]
    fn corpus_does_not_grow_after_bug_found() {
        let tmp = tempfile::tempdir().unwrap();
        let corpus_dir = tmp.path().join("corpus");

        // First run: the fuzzer finds the bug and adds items.
        run(make_args(corpus_dir.clone())).expect("first run should succeed");
        let count_after_first = count_corpus_files(&corpus_dir);
        assert!(
            count_after_first > 0,
            "corpus should have items after first run"
        );

        // Second run: the fuzzer should not add redundant items.
        run(make_args(corpus_dir.clone())).expect("second run should succeed");
        let count_after_second = count_corpus_files(&corpus_dir);
        assert_eq!(
            count_after_first, count_after_second,
            "corpus should not grow after bug is already found"
        );
    }
}
