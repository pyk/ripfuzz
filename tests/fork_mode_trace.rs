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
use raptor::{
    Artifact, ArtifactId, Chain, ChainConfig, Contract, DeployInput, ForkDBConfig, MockTransport,
    Project, TraceContext, Transaction,
};
use revm::primitives::Bytes;
use serde_json::json;

// ---------------------------------------------------------------------------
// Fork mode constants (Base mainnet)
// ---------------------------------------------------------------------------

/// Block number used for fork mode.
const BLOCK_NUMBER: u64 = 47_531_700;

/// Live RPC URL for populating the fork cache.
const LIVE_RPC_URL: &str = "https://base-rpc.publicnode.com";

/// Cache directory for fork DB responses.
const CACHE_DIR: &str = "fixtures/fork-mode-trace/rpc";

fn block_json() -> serde_json::Value {
    json!({
        "number": "0x2d546b4",
        "timestamp": "0x6a34ea4b",
        "hash": "0x0133fd6ac4a984e9641549d15439d9fec92b3ef8720da58d37bc7aa7b7bc14bc",
        "miner": "0x4200000000000000000000000000000000000011",
        "gasLimit": "0x17d78400",
        "baseFeePerGas": "0x4c4b40",
        "difficulty": "0x0",
        "mixHash": "0xb848fb237cee32613f8e5dfe85b4247f4ad8444d5b534c96a5cff3f47387987d",
        "excessBlobGas": "0x0"
    })
}

fn mock_fork_setup(transport: &MockTransport, url: &str) {
    let chain_id_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
    ]);
    let block_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[format!("0x{BLOCK_NUMBER:x}"), false]}
    ]);
    transport.mock_response(
        url,
        &chain_id_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":"0x2105"}]),
    );
    transport.mock_response(
        url,
        &block_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":block_json()}]),
    );
}

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
// Handler contract interface
// ---------------------------------------------------------------------------

alloy_sol_types::sol! {
    interface ISupplyUSDC {
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
/// fork mode with an on-disk response cache.  The cache must be
/// populated before running this test — see the `populate_fork_cache`
/// test below or run:
///
/// ```sh
/// cargo test --test fork_mode_trace populate_fork_cache -- --nocapture --ignored
/// ```
///
/// The handler contract is a local deployment; external projects
/// (Pool Proxy + Implementation + aUSDC) provide ABIs and labels so
/// that the trace renders contract names and decoded calldata.
#[test]
fn supply_usdc_to_aave_v3_pool() {
    // Require the disk cache to exist so the test doesn't silently pass
    // with incomplete state.
    let cache_path = PathBuf::from(CACHE_DIR);
    assert!(
        cache_path.exists(),
        "fork cache not found at {} — run `cargo test --test fork_mode_trace populate_fork_cache -- --ignored` first",
        cache_path.display(),
    );

    let transport = MockTransport::default();
    let url = "mock://test";

    mock_fork_setup(&transport, url);

    let config = ForkDBConfig::new(url)
        .block_number(BLOCK_NUMBER)
        .cache_dir(CACHE_DIR);
    let mut chain = Chain::fork_with_transport(
        ChainConfig::default().trace(true),
        config,
        transport.clone(),
    )
    .unwrap();

    run_supply_test(&mut chain);
}

/// Populate the fork-mode disk cache by running the supply flow against
/// the live Base RPC.  This test is `#[ignore]` by default; run it
/// explicitly when you need to refresh the cached on-chain data.
#[test]
#[ignore]
fn populate_fork_cache() {
    let config = ForkDBConfig::new(LIVE_RPC_URL)
        .block_number(BLOCK_NUMBER)
        .cache_dir(CACHE_DIR);
    let mut chain = Chain::fork_with_transport(
        ChainConfig::default().trace(true),
        config,
        ureq::Agent::new_with_defaults(),
    )
    .unwrap();

    run_supply_test(&mut chain);
}

/// Shared test body: deploy handler, execute supply, format trace.
fn run_supply_test(chain: &mut Chain) {
    // 1. Deploy handler contract
    let handler_project = Project::new("fixtures/fork-mode-trace");
    let handler_artifacts = handler_project.load_artifacts().unwrap();
    let handler_id = ArtifactId::try_from("src/SupplyUSDC.sol:SupplyUSDC").unwrap();
    let handler_contract = Contract::try_get(&handler_artifacts, &handler_id).unwrap();

    let deployment = chain
        .deploy(DeployInput::new(&handler_contract.initcode))
        .unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    // 2. Execute supply
    let supply_calldata = Bytes::from(ISupplyUSDC::supplyCall::new(()).abi_encode());
    let txs = [Transaction::new(target).calldata(supply_calldata)];
    let exec_output = chain.exec(&txs).unwrap();

    // 4. Format trace with external project context
    let ctx = build_trace_context(target);
    let trace = exec_output.trace.as_ref().expect("trace must be present");
    let formatted = format!("{}", trace.display_with(&ctx));

    // Write the formatted trace to the output file for review.
    let output_path = "fixtures/fork-mode-trace/outputs/supply_usdc.txt";
    std::fs::write(output_path, &formatted).unwrap();

    assert!(!formatted.is_empty(), "trace output must not be empty");
}
