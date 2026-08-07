//! Integration tests: EVM tracer in fork mode with external projects.
//!
//! Validates that the EVM tracer produces human-readable traces when
//! executing against real on-chain contracts in fork mode. External
//! project artifacts (e.g. Aave V3 Pool) are used to resolve labels
//! and decode calls in the trace output.

use std::collections::HashMap;
use std::path::PathBuf;

use alloy_primitives::Address;
use alloy_sol_types::SolCall;
use revm::primitives::Bytes;
use ripfuzz::{
    Artifact, ArtifactId, Chain, ChainConfig, Contract, DeployInput, ForkDBConfig, Project,
    SetupInput, TraceContext, Transaction,
};

// ---------------------------------------------------------------------------
// Fork mode constants (Base mainnet)
// ---------------------------------------------------------------------------

/// Block number used for fork mode.
const BLOCK_NUMBER: u64 = 47_664_508;

/// RPC URL used as the cache key namespace for fork DB responses.
const LIVE_RPC_URL: &str = "https://base-rpc.publicnode.com";

/// Cache directory for fork DB responses.
const CACHE_DIR: &str = "fixtures/fork-mode-trace/rpc";

// ---------------------------------------------------------------------------
// On-chain addresses
// ---------------------------------------------------------------------------

/// Base USDC token.
const USDC_ADDRESS: Address =
    alloy_primitives::address!("833589fCD6eDb6E08f4c7C32D4f71b54bdA02913");

/// Aave V3 Pool Proxy on Base.
const POOL_ADDRESS: Address =
    alloy_primitives::address!("A238Dd80C259a72e81d7E4664a9801593F98d1c5");

/// Aave V3 Pool Implementation on Base.
const POOL_IMPL_ADDRESS: Address =
    alloy_primitives::address!("a4abc5fcba6d0d7e3d144d6dbf6cb6128599dfdb");

/// aUSDC token (aToken proxy for USDC on Base).
const AUSDC_ADDRESS: Address =
    alloy_primitives::address!("59dca05b6c26dbd64b5381374aaac5cd05644c28");

/// aUSDC implementation.
const AUSDC_IMPL_ADDRESS: Address =
    alloy_primitives::address!("7354dc700a1a2ab9622f2292b60ca1ced5b204d0");

// ---------------------------------------------------------------------------
// Harness contract interface
// ---------------------------------------------------------------------------

alloy_sol_types::sol! {
    interface ISupplyUSDC {
        function setup() external;
        function supply() external;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load all artifacts from a project and insert them into the given map.
fn load_project_artifacts(artifacts: &mut HashMap<ArtifactId, Artifact>, project_path: &str) {
    let project = Project::new(project_path);
    let loaded = project.load_artifacts().unwrap();
    artifacts.extend(loaded);
}

/// Build a merged `TraceContext` from the handler project and all
/// external projects, with labels for known on-chain addresses.
fn build_trace_context(handler_address: Address) -> TraceContext {
    let mut all_artifacts: HashMap<ArtifactId, Artifact> = HashMap::new();
    load_project_artifacts(&mut all_artifacts, "fixtures/fork-mode-trace");
    load_project_artifacts(&mut all_artifacts, "fixtures/aave-v3-pool-proxy");
    load_project_artifacts(&mut all_artifacts, "fixtures/aave-v3-pool-implementation");
    load_project_artifacts(&mut all_artifacts, "fixtures/aave-v3-ausdc-proxy");
    load_project_artifacts(&mut all_artifacts, "fixtures/aave-v3-ausdc-implementation");

    TraceContext::from_artifacts(all_artifacts)
        .with_label(handler_address, "SupplyUSDC")
        .with_label(USDC_ADDRESS, "USDC")
        .with_label(POOL_ADDRESS, "AaveV3Pool")
        .with_label(POOL_IMPL_ADDRESS, "PoolInstance")
        .with_label(AUSDC_ADDRESS, "aUSDC")
        .with_label(AUSDC_IMPL_ADDRESS, "ATokenInstance")
}

// ---------------------------------------------------------------------------
// Test: supply USDC to Aave V3 pool (using disk cache)
// ---------------------------------------------------------------------------

/// Integration test: supply USDC to the Aave V3 pool on Base using
/// fork mode with an on-disk response cache.  The cache is committed
/// to the repository so no live RPC is needed.
///
/// The harness contract is a local deployment; external projects
/// (Pool Proxy + Implementation + aUSDC) provide ABIs and labels so
/// that the trace renders contract names and decoded calldata.
#[test]
fn supply_usdc_to_aave_v3_pool() {
    // Require the disk cache to exist so the test doesn't silently pass
    // with incomplete state.
    let cache_path = PathBuf::from(CACHE_DIR);
    assert!(
        cache_path.exists(),
        "fork cache not found at {}",
        cache_path.display(),
    );

    // Empty sandbox; harness setup calls rvm.fork with the cache defaults.
    // LIVE_RPC_URL is only the cache key namespace (must match fixture setup).
    let _ = LIVE_RPC_URL;
    let _ = BLOCK_NUMBER;
    let fork_defaults = ForkDBConfig::new("").cache_dir(CACHE_DIR);
    let mut chain = Chain::new(
        ChainConfig::default()
            .trace(true)
            .with_fork_defaults(fork_defaults),
    )
    .unwrap();

    // 1. Deploy harness contract
    let handler_project = Project::new("fixtures/fork-mode-trace");
    let handler_artifacts = handler_project.load_artifacts().unwrap();
    let handler_id = ArtifactId::try_from("src/SupplyUSDC.sol:SupplyUSDC").unwrap();
    let harness_contract = Contract::try_get(&handler_artifacts, &handler_id).unwrap();

    let deployment = chain
        .deploy(DeployInput::new(&harness_contract.initcode))
        .unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    // 2. setup: rvm.fork pins Base state from the on-disk cache
    let setup = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup.result.success, "setup (rvm.fork) must succeed");

    // 3. Execute supply
    let supply_calldata = Bytes::from(ISupplyUSDC::supplyCall::new(()).abi_encode());
    let txs = [Transaction::new(target).calldata(supply_calldata)];
    let exec_output = chain.exec(&txs).unwrap();

    // 4. Format trace with external project context
    let ctx = build_trace_context(target);
    let trace = exec_output.trace.as_ref().expect("trace must be present");
    let formatted = format!("{}", trace.display_with(&ctx));

    // Compare trace output against the golden file.
    let expected_path = "fixtures/fork-mode-trace/expected/supply_usdc.txt";
    let expected = std::fs::read_to_string(expected_path)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "trace output must match expected"
    );
}
