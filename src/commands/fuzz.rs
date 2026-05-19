//! `fuzz` CLI command implementation.

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use alloy_primitives::Address;
use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use revm::primitives::U256;
use tracing::{debug, info, instrument};

use crate::campaign::CampaignConfig;
use crate::chain::Environment;
use crate::contract::ContractBuilder;
use crate::contract::resolve_coverage_to_source;
use crate::rpc::RpcClient;

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

    /// Fork mode configuration.
    #[command(flatten)]
    pub fork: ForkArgs,

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

#[derive(Debug, Parser)]
pub struct ForkArgs {
    /// JSON-RPC URL to fork from (e.g. https://eth.llamarpc.com).
    /// Repeatable for multiple endpoints of the same chain.
    #[arg(long = "fork-rpc-url", value_name = "URL", help_heading = "Fork Mode", action = clap::ArgAction::Append)]
    pub rpc_urls: Vec<String>,

    /// Block number to fork at. Must be <= the remote latest block.
    #[arg(long = "fork-rpc-block", value_name = "N", help_heading = "Fork Mode")]
    pub rpc_block: Option<u64>,

    /// Number of concurrent RPC connections for fork mode.
    #[arg(
        long = "fork-rpc-pool",
        default_value = "4",
        value_name = "N",
        help_heading = "Fork Mode"
    )]
    pub rpc_pool: u32,

    /// Maximum retry attempts per RPC URL after transient failure.
    #[arg(
        long = "fork-rpc-retries",
        default_value = "3",
        value_name = "N",
        help_heading = "Fork Mode"
    )]
    pub rpc_retries: u32,

    /// Initial retry backoff in milliseconds (doubles each attempt).
    #[arg(
        long = "fork-rpc-backoff",
        default_value = "100",
        value_name = "MS",
        help_heading = "Fork Mode"
    )]
    pub rpc_backoff: u64,

    /// Optional rate limit: maximum requests per second across all URLs.
    #[arg(
        long = "fork-rpc-rate-limit",
        value_name = "N",
        help_heading = "Fork Mode"
    )]
    pub rpc_rate_limit: Option<u64>,

    /// Request timeout in milliseconds for each RPC call.
    #[arg(
        long = "fork-rpc-timeout",
        default_value = "30000",
        value_name = "MS",
        help_heading = "Fork Mode"
    )]
    pub rpc_timeout: u64,
}

impl Default for ForkArgs {
    fn default() -> Self {
        Self {
            rpc_urls: Vec::new(),
            rpc_block: None,
            rpc_pool: 4,
            rpc_retries: 3,
            rpc_backoff: 100,
            rpc_rate_limit: None,
            rpc_timeout: 30000,
        }
    }
}

impl ForkArgs {
    /// Validate that every provided RPC URL returns the same chain_id.
    pub fn validate_chain_id(&self, project_path: impl AsRef<Path>) -> Result<u64> {
        let project_path = project_path.as_ref();
        let mut ids: Vec<(&str, u64)> = Vec::new();

        for url in self.rpc_urls.iter().collect::<HashSet<&String>>() {
            info!(target: "raptor::user", %url, timeout = "5s", "Fetching chain_id");
            let url_t0 = std::time::Instant::now();
            let chain_id = crate::rpc::get_chain_id(project_path, url)?;
            let url_elapsed = url_t0.elapsed();
            info!(target: "raptor::user", %url, chain_id, took = %format!("{}ms", url_elapsed.as_millis()), "OK");
            ids.push((url, chain_id));
        }

        ensure!(!ids.is_empty(), "no RPC URLs to validate");

        let first = ids[0].1;
        if ids.iter().all(|(_, id)| *id == first) {
            Ok(first)
        } else {
            let details = ids
                .iter()
                .map(|(url, id)| format!("{} -> {}", url, id))
                .collect::<Vec<String>>()
                .join(", ");
            bail!("chain ID mismatch: {}", details);
        }
    }

    /// Build the RPC client and validate the fork block.
    pub fn build_rpc(&self, chain_id: u64) -> Result<(std::sync::Arc<dyn RpcClient>, u64)> {
        let block = self
            .rpc_block
            .context("--fork-rpc-block is required with --fork-rpc-url")?;
        let rpc_instance = crate::rpc::Rpc::with_urls(&self.rpc_urls)
            .with_pool_size(self.rpc_pool)
            .with_retries(self.rpc_retries)
            .with_retry_backoff(std::time::Duration::from_millis(self.rpc_backoff))
            .with_requests_per_second(self.rpc_rate_limit)
            .with_timeout(std::time::Duration::from_millis(self.rpc_timeout))
            .with_chain_id(chain_id)
            .build()?;
        info!(target: "raptor::user", urls = ?self.rpc_urls, "Fetching latest block");
        let t0 = std::time::Instant::now();
        let latest = rpc_instance
            .latest_block_number()
            .context("failed to query latest block")?;
        let elapsed = t0.elapsed();
        info!(target: "raptor::user", block = latest, took = %format!("{}ms", elapsed.as_millis()), "OK");
        if block > latest {
            bail!("--fork-rpc-block ({block}) exceeds remote latest block ({latest})");
        }
        info!(target: "raptor::user", urls = ?self.rpc_urls, block = block, "Forking");
        Ok((std::sync::Arc::new(rpc_instance), block))
    }
}

#[instrument(skip(args), fields(target = ?args.target_path, threads = args.threads, max_runs = args.max_runs))]
pub fn run(args: Args) -> Result<()> {
    // Resolve project path
    let project_path = args.project_path.map(Ok).unwrap_or_else(env::current_dir)?;
    debug!(?project_path, "resolved project path");

    // Compile target
    info!(target: "raptor::user", project = %project_path.display(), target = %args.target_path.display(), "Compiling");
    let t0 = std::time::Instant::now();
    let artifact = ContractBuilder::for_project(&project_path)
        .with_target_path(&args.target_path)
        .build()?;
    let compile_elapsed = t0.elapsed();
    info!(target: "raptor::user", took = %format!("{}ms", compile_elapsed.as_millis()), "Finished compiling target");

    // Validate artifact
    let targets: Vec<String> = artifact.target_functions().map(|f| f.signature()).collect();
    ensure!(
        !targets.is_empty(),
        "No target functions found in target contract"
    );
    let target_list = targets
        .iter()
        .enumerate()
        .map(|(i, s)| format!("         {}. {s}", i + 1))
        .collect::<Vec<String>>()
        .join("\n");
    info!(target: "raptor::user", "Found {} target functions\n{}", targets.len(), target_list);

    let invariant_list = artifact
        .invariants
        .iter()
        .enumerate()
        .map(|(i, (_, name))| format!("         {}. {name}()", i + 1))
        .collect::<Vec<String>>()
        .join("\n");
    info!(target: "raptor::user", "Found {} invariants\n{}", artifact.invariants.len(), invariant_list);

    // Build environment
    let env = if args.fork.rpc_urls.is_empty() {
        Environment::sandbox()
    } else {
        let chain_id = args.fork.validate_chain_id(&project_path)?;
        let (rpc, block) = args.fork.build_rpc(chain_id)?;
        Environment::fork(rpc, block, &project_path)
    };

    let config = CampaignConfig {
        threads: args.threads,
        max_runs: args.max_runs,
        timeout_secs: args.timeout_secs,
        sequence_length: args.sequence_length,
        seed: args.seed,
        max_block_number_delay: args.max_block_number_delay,
        max_block_timestamp_delay: args.max_block_timestamp_delay,
    };

    info!(target: "raptor::user", threads = args.threads, seed = args.seed, max_runs = args.max_runs, seq_length = args.sequence_length, timeout_secs = args.timeout_secs.unwrap_or(0), "Fuzzing configuration");

    // Build chain
    let vm = crate::vm::Vm::new(
        crate::vm::VmConfig::default()
            .with_ffi(args.ffi)
            .with_project_root(&project_path),
    );
    let chain = crate::chain::Chain::for_artifact(&artifact)
        .with_project(&project_path)
        .with_vm(vm)
        .with_deploy_value(args.deploy_value)
        .with_deployer(args.deployer_address)
        .with_environment(env)
        .init()?
        .setup()?;

    // Build campaign
    let sequence_length = config.sequence_length;
    let mut builder = crate::campaign::CampaignBuilder::new()
        .with_chain(chain)
        .with_config(config)
        .with_fuzzer(crate::fuzzer::DefaultFuzzerFactory);

    if let Some(ref dir) = args.corpus_dir {
        let seeds = crate::campaign::build_seeds(&artifact, sequence_length);
        let corpus = match crate::corpus::Corpus::load(dir) {
            Ok(mut c) => {
                c.set_storage_dir(dir);
                c
            }
            Err(_) => {
                let mut c = crate::corpus::Corpus::with_seeds(seeds);
                c.set_storage_dir(dir);
                c
            }
        };
        builder = builder.with_corpus(std::sync::Arc::new(std::sync::RwLock::new(corpus)));
    }

    let campaign = builder.with_artifact(artifact).build()?;

    // Run campaign
    let result = campaign.run()?;

    let artifact = campaign.artifact();

    // Aggregate results
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

    // Report campaign result
    info!(target: "raptor::user", runs = result.runs, calls = result.total_calls, "Fuzzing completed");
    info!(target: "raptor::user", calls_per_sec = calls_per_sec, "Throughput");
    info!(target: "raptor::user", avg_gas_per_call = avg_gas_per_call, "Average gas per call");
    if result.failures.is_empty() {
        info!(target: "raptor::user", "All invariants passed");
    } else {
        for failure in &result.failures {
            info!(target: "raptor::user", "");
            info!(target: "raptor::user", contract = %artifact.contract_name, test = %failure.function_name, "[FAILED] Invariant Test");
            info!(target: "raptor::user", contract = %artifact.contract_name, test = %failure.function_name, "Test failed after the following call sequence");
            info!(target: "raptor::user", "[Call Sequence]");
            info!(target: "raptor::user", "{}", crate::fuzzer::format_failure(artifact, failure, args.deployer_address));
        }
        let total = artifact.invariants.len();
        let failed = result.failures.len();
        let passed = total.saturating_sub(failed);
        info!(target: "raptor::user", "");
        info!(target: "raptor::user", passed = passed, failed = failed, "Test summary");
    }

    let report = resolve_coverage_to_source(&result.coverage, artifact);
    if report.hit_count() > 0 {
        info!(target: "raptor::user", hits = report.hit_count(), "Coverage summary");
    } else {
        info!(target: "raptor::user", hits = 0, "Coverage summary");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::fs::{create_dir_all, write};
    use std::hash::{Hash, Hasher};
    use std::path::Path;

    use alloy_primitives::Address;
    use revm::primitives::U256;

    use super::{ForkArgs, parse_address, parse_balance};

    fn seed_chain_id_cache(project_path: impl AsRef<Path>, url: &str, chain_id: u64) {
        let project_path = project_path.as_ref();
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        let url_hash = format!("{:x}", hasher.finish());

        let cache_dir = project_path.join("raptor").join("cache").join("chain_id");
        create_dir_all(&cache_dir).unwrap();
        write(cache_dir.join(&url_hash), format!("0x{:x}", chain_id)).unwrap();
    }

    #[test]
    fn validate_chain_id_success_when_all_match() {
        let tmp = tempfile::tempdir().unwrap();
        let args = ForkArgs {
            rpc_urls: vec!["http://a.com".into(), "http://b.com".into()],
            rpc_block: Some(1),
            ..ForkArgs::default()
        };
        seed_chain_id_cache(tmp.path(), "http://a.com", 1);
        seed_chain_id_cache(tmp.path(), "http://b.com", 1);
        assert_eq!(args.validate_chain_id(tmp.path()).unwrap(), 1);
    }

    #[test]
    fn validate_chain_id_fails_on_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let args = ForkArgs {
            rpc_urls: vec!["http://a.com".into(), "http://b.com".into()],
            rpc_block: Some(1),
            ..ForkArgs::default()
        };
        seed_chain_id_cache(tmp.path(), "http://a.com", 1);
        seed_chain_id_cache(tmp.path(), "http://b.com", 56);
        let err = args.validate_chain_id(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("chain ID mismatch"));
    }

    #[test]
    fn validate_chain_id_dedups_duplicate_urls() {
        let tmp = tempfile::tempdir().unwrap();
        let args = ForkArgs {
            rpc_urls: vec!["http://a.com".into(), "http://a.com".into()],
            rpc_block: Some(1),
            ..ForkArgs::default()
        };
        seed_chain_id_cache(tmp.path(), "http://a.com", 1);
        assert_eq!(args.validate_chain_id(tmp.path()).unwrap(), 1);
    }

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
