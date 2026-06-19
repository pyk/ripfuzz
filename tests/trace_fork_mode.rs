//! Integration tests: EVM tracer in fork mode with external projects.
//!
//! Validates that the EVM tracer produces human-readable traces when
//! executing against real on-chain contracts in fork mode. External
//! project artifacts (e.g. Aave V3 Pool) are used to resolve labels
//! and decode calls in the trace output.

use std::collections::HashMap;

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

/// Base chain ID (used in fork init).
const _CHAIN_ID: u64 = 8453;

/// Block number used for fork mode.
const BLOCK_NUMBER: u64 = 47_531_700;

/// Block timestamp at block 47_531_700.
const _BLOCK_TIMESTAMP: u64 = 1_783_472_747;

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

fn fork_chain(transport: &MockTransport, url: &str) -> Chain {
    mock_fork_setup(transport, url);
    let config = ForkDBConfig::new(url).block_number(BLOCK_NUMBER);
    Chain::fork_with_transport(
        ChainConfig::default().trace(true),
        config,
        transport.clone(),
    )
    .unwrap()
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
    load_project_artifacts(&mut all_artifacts, "fixtures/trace-fork-mode");
    load_project_artifacts(&mut all_artifacts, "fixtures/aave-v3-pool-proxy");
    load_project_artifacts(&mut all_artifacts, "fixtures/aave-v3-pool-implementation");

    TraceContext::from_artifacts(all_artifacts)
        .with_label(handler_address, "SupplyUSDC")
        .with_label(USDC_ADDRESS, "USDC")
        .with_label(POOL_ADDRESS, "AaveV3Pool")
        .with_label(POOL_IMPL_ADDRESS, "PoolInstance")
}

// ---------------------------------------------------------------------------
// Test: supply USDC to Aave V3 pool
// ---------------------------------------------------------------------------

/// Integration test: supply USDC to the Aave V3 pool on Base using
/// fork mode. The handler contract is a local deployment; external
/// projects (Pool Proxy + Implementation) provide ABIs and labels so
/// that the trace renders contract names and decoded calldata.
///
/// The mock transport starts empty and is populated iteratively with
/// real on-chain data fetched via `cast` from Base mainnet at the
/// specified block number.
#[test]
fn supply_usdc_to_aave_v3_pool() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = fork_chain(&transport, url);

    assert_eq!(
        transport.total_calls(),
        2,
        "fork init must fetch chain_id and block"
    );

    // ------------------------------------------------------------------
    // 1. Deploy handler contract
    // ------------------------------------------------------------------
    let handler_project = Project::new("fixtures/trace-fork-mode");
    let handler_artifacts = handler_project.load_artifacts().unwrap();
    let handler_id = ArtifactId::try_from("src/SupplyUSDC.sol:SupplyUSDC").unwrap();
    let handler_contract = Contract::try_get(&handler_artifacts, &handler_id).unwrap();

    let deployment = chain
        .deploy(DeployInput::new(&handler_contract.initcode))
        .unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    // ------------------------------------------------------------------
    // 2. Mock on-chain data for the supply call
    // ------------------------------------------------------------------

    // 2a. USDC proxy account (0x833589... on Base)
    {
        let usdc_code = include_str!("../fixtures/trace-fork-mode/bytecodes/usdc.hex")
            .trim()
            .trim_end_matches('\n');
        let addr_hex = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":[addr_hex, block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x22d1a2c0d26ed3"},
                {"jsonrpc":"2.0","id":1,"result":"0x1"},
                {"jsonrpc":"2.0","id":2,"result": usdc_code}
            ]),
        );
    }

    // 2b. USDC implementation account (0x2Ce6311d...)
    {
        let usdc_impl_code = include_str!("../fixtures/trace-fork-mode/bytecodes/usdc-impl.hex")
            .trim()
            .trim_end_matches('\n');
        let addr_hex = "0x2ce6311ddae708829bc0784c967b7d77d19fd779";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":[addr_hex, block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0"},
                {"jsonrpc":"2.0","id":1,"result":"0x1"},
                {"jsonrpc":"2.0","id":2,"result": usdc_impl_code}
            ]),
        );
    }

    // 2c. USDC proxy: implementation slot
    // slot = keccak256("org.zeppelinos.proxy.implementation")
    //        = 0x7050c9e0f4ca769c69bd3a8ef740bc37934f8e2c036e5a723fd8ee048ed3f8c3
    // value = 0x2Ce6311ddAE708829bc0784C967b7d77D19FD779 (USDC impl)
    {
        let proxy_hex = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":[proxy_hex, "0x7050c9e0f4ca769c69bd3a8ef740bc37934f8e2c036e5a723fd8ee048ed3f8c3", block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0000000000000000000000002ce6311ddae708829bc0784c967b7d77d19fd779"}
            ]),
        );
    }

    // 2d. USDC: allowance slot for handler -> pool (return 0, fresh approval)
    {
        let proxy_hex = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":[proxy_hex, "0x10d6a54a4754c8869d6886b5f5d7fbfa5b4522237ea5c60d11bc4e7a1ff9390b", block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0000000000000000000000000000000000000000000000000000000000000000"}
            ]),
        );
    }

    // 2e. USDC: state slot 1
    {
        let proxy_hex = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":[proxy_hex, "0x1", block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x000000000000000000000000d3571b3bc51cecff49194ad67afffc648d5e07b4"}
            ]),
        );
    }

    // 2f. USDC: role/permit slot
    {
        let proxy_hex = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":[proxy_hex, "0x72345559ee102c4e69b95c1745c69df25d7e3bd353fb1642ff14b7dabbe7d5df", block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0000000000000000000000000000000000000000000000000000000000000000"}
            ]),
        );
    }

    // 2g. Aave V3 Pool Proxy account (0xA238Dd80...)
    {
        let pool_code = include_str!("../fixtures/trace-fork-mode/bytecodes/pool-proxy.hex")
            .trim()
            .trim_end_matches('\n');
        let addr_hex = "0xa238dd80c259a72e81d7e4664a9801593f98d1c5";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":[addr_hex, block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0"},
                {"jsonrpc":"2.0","id":1,"result":"0x1"},
                {"jsonrpc":"2.0","id":2,"result": pool_code}
            ]),
        );
    }

    // 2h. Aave V3 Pool Proxy: implementation slot (ERC-1967)
    {
        let proxy_hex = "0xa238dd80c259a72e81d7e4664a9801593f98d1c5";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":[proxy_hex, "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc", block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x000000000000000000000000a4abc5fcba6d0d7e3d144d6dbf6cb6128599dfdb"}
            ]),
        );
    }

    // 2i. Aave V3 Pool Implementation account (0xa4abc5fc...)
    {
        let pool_impl_code = include_str!("../fixtures/trace-fork-mode/bytecodes/pool-impl.hex")
            .trim()
            .trim_end_matches('\n');
        let addr_hex = "0xa4abc5fcba6d0d7e3d144d6dbf6cb6128599dfdb";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":[addr_hex, block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0"},
                {"jsonrpc":"2.0","id":1,"result":"0x1"},
                {"jsonrpc":"2.0","id":2,"result": pool_impl_code}
            ]),
        );
    }

    // 2j. Pool: configurator slot
    {
        let proxy_hex = "0xa238dd80c259a72e81d7e4664a9801593f98d1c5";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":[proxy_hex, "0x41e584e805d183e6ebdd4ffa3391b4e09552e4a875e1427345ccff3efc7ff0e4", block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0000000000000000000000000000000000000000000000000000000000000000"}
            ]),
        );
    }

    // 2k. Library at 0x584c7d8c... (delegatecall from Pool impl)
    {
        let lib_code = include_str!("../fixtures/trace-fork-mode/bytecodes/lib-584c.hex")
            .trim()
            .trim_end_matches('\n');
        let addr_hex = "0x584c7d8c4cb05304fe5ac7fbc97f20a10fb07564";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":[addr_hex, block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0"},
                {"jsonrpc":"2.0","id":1,"result":"0x1"},
                {"jsonrpc":"2.0","id":2,"result": lib_code}
            ]),
        );
    }

    // 2l. Pool: reserve data slots (USDC config struct)
    {
        let proxy_hex = "0xa238dd80c259a72e81d7e4664a9801593f98d1c5";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let slots = [
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b658",
                "0x100000000000000000000003e800db5858000c5691c003e8850629041e781d4c",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b659",
                "0x000000000019e78a30dca915b70a80620000000003ac26ba8675d0627126114d",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b65a",
                "0x00000000002280a6f96f48805d2aed630000000003d89c116be9f5601f663c83",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b65b",
                "0x0000000000000000000004006a34ef4d00000000000000000000000001d6fda4",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b65c",
                "0x0000000000000000000000004e65fe4dba92790696d040ac24aa414708f5c0ab",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b65d",
                "0x0000000000000000000000000000000000000000000000000000000000000000",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b65e",
                "0x00000000000000000000000059dca05b6c26dbd64b5381374aaac5cd05644c28",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b65f",
                "0x00000000000000000000000086ab1c62a8bf868e1b3e1ab87d587aba6fbcbdc5",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b660",
                "0x000000000000000000001ab59b6d05dc000000000000000000000000055e48a2",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b661",
                "0x0000000000000000000000000000000000000000000000000000000000000000",
            ),
            (
                "0x33a59812d7f150f2c2e7cf398df161b5d00b06dc197e974636ff8a741412b662",
                "0x0000000000000000000000000000000000000000000000000000000000000000",
            ),
        ];
        for (slot_hex, result_hex) in &slots {
            let payload = json!([
                {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":[proxy_hex, slot_hex, block_hex]}
            ]);
            transport.mock_response(
                url,
                &payload,
                json!([
                    {"jsonrpc":"2.0","id":0,"result": result_hex}
                ]),
            );
        }
    }

    // 2m. Aave aToken proxy (0x59dca05b... from reserve data slot 0x65e)
    {
        let atoken_code = include_str!("../fixtures/trace-fork-mode/bytecodes/atoken-59dc.hex")
            .trim()
            .trim_end_matches('\n');
        let addr_hex = "0x59dca05b6c26dbd64b5381374aaac5cd05644c28";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":[addr_hex, block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0"},
                {"jsonrpc":"2.0","id":1,"result":"0x1"},
                {"jsonrpc":"2.0","id":2,"result": atoken_code}
            ]),
        );
    }

    // 2n. aToken proxy: implementation slot (ERC-1967)
    {
        let proxy_hex = "0x59dca05b6c26dbd64b5381374aaac5cd05644c28";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":[proxy_hex, "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc", block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0000000000000000000000007354dc700a1a2ab9622f2292b60ca1ced5b204d0"}
            ]),
        );
    }

    // 2o. aToken implementation account (0x7354dc70...)
    {
        let atoken_impl_code =
            include_str!("../fixtures/trace-fork-mode/bytecodes/atoken-impl.hex")
                .trim()
                .trim_end_matches('\n');
        let addr_hex = "0x7354dc700a1a2ab9622f2292b60ca1ced5b204d0";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":[addr_hex, block_hex]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":[addr_hex, block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0"},
                {"jsonrpc":"2.0","id":1,"result":"0x1"},
                {"jsonrpc":"2.0","id":2,"result": atoken_impl_code}
            ]),
        );
    }

    // 2p. aToken: storage slot 0x3a
    {
        let proxy_hex = "0x59dca05b6c26dbd64b5381374aaac5cd05644c28";
        let block_hex = format!("0x{BLOCK_NUMBER:x}");
        let payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":[proxy_hex, "0x3a", block_hex]}
        ]);
        transport.mock_response(
            url,
            &payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x000000000000000000000000000000000000000000000000000070e4aae32790"}
            ]),
        );
    }

    // ------------------------------------------------------------------
    // 3. Execute supply
    // ------------------------------------------------------------------
    let supply_calldata = Bytes::from(ISupplyUSDC::supplyCall::new(()).abi_encode());
    let txs = [Transaction::new(target).calldata(supply_calldata)];
    let exec_output = chain.exec(&txs).unwrap();

    // ------------------------------------------------------------------
    // 4. Format trace with external project context
    // ------------------------------------------------------------------
    let ctx = build_trace_context(target);
    let trace = exec_output.trace.as_ref().expect("trace must be present");
    let formatted = format!("{}", trace.display_with(&ctx));

    // Write the formatted trace to the output file for review.
    let output_path = "fixtures/trace-fork-mode/outputs/supply_usdc.txt";
    std::fs::write(output_path, &formatted).unwrap();

    // For now, just check that the trace is non-empty.
    // Once the output is reviewed and stabilized, replace this with
    // an exact assertion against the expected file content.
    assert!(!formatted.is_empty(), "trace output must not be empty");
}
