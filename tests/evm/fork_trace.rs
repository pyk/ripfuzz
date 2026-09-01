//! Integration tests: EVM tracer in fork mode.
//!
//! Validates that the EVM tracer produces human-readable traces when
//! executing against real on-chain contracts in fork mode. Decoding uses
//! the compiled harness solc output plus explicit address labels.

use std::path::PathBuf;

use alloy_primitives::Address;
use alloy_sol_types::SolCall;
use revm::primitives::Bytes;
use ripfuzz::compilers::solc::{Solc, SolcOutput};
use ripfuzz::harness::HarnessId;
use ripfuzz::{
    Chain, ChainConfig, DeployInput, ForkDBConfig, SetupInput, TraceContext, Transaction,
};

// ---------------------------------------------------------------------------
// Fork mode constants (Base mainnet)
// ---------------------------------------------------------------------------

/// Block number used for fork mode.
const BLOCK_NUMBER: u64 = 47_664_508;

/// RPC URL used as the cache key namespace for fork DB responses.
const LIVE_RPC_URL: &str = "https://base-rpc.publicnode.com";

/// Cache directory for fork DB responses.
const CACHE_DIR: &str = "fixtures/evm/fork-trace/rpc";

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

fn compile_fixture(root: &str, target: &str) -> SolcOutput {
    let id = HarnessId::try_from(target).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    Solc::new()
        .with_version("0.8.36")
        .with_root(root)
        .with_target(&id.path)
        .with_name(&id.name)
        .with_out(tmp.path().join("out"))
        .compile()
        .unwrap_or_else(|err| panic!("fixture `{target}` must compile: {err}"))
}

/// Build a `TraceContext` from the compiled harness plus on-chain labels.
fn build_trace_context(handler_address: Address, solc_output: &SolcOutput) -> TraceContext {
    TraceContext::from(solc_output)
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
/// The harness contract is a local deployment. Address labels name the
/// on-chain Aave and USDC contracts; call decoding uses the harness
/// solc output only.
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
    let solc_output = compile_fixture("fixtures/evm/fork-trace", "SupplyUSDC.sol:SupplyUSDC");
    let deployment = chain
        .deploy(DeployInput::new(solc_output.initcode().unwrap()))
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

    // 4. Format trace from the harness solc output plus address labels.
    let ctx = build_trace_context(target, &solc_output);
    let trace = exec_output.trace.as_ref().expect("trace must be present");
    let formatted = format!("{}", trace.display_with(&ctx));

    let expected_path = "fixtures/evm/fork-trace/expected/supply_usdc.txt";
    let expected = std::fs::read_to_string(expected_path)
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
    assert_eq!(
        formatted.trim(),
        expected.trim(),
        "trace output must match expected"
    );
}
