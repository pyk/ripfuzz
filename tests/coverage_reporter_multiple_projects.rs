//! Integration test: multi-project coverage report in fork mode.
//!
//! Validates that `--external-project` artifacts are matched against
//! fork-mode bytecodes and that their source files appear in the
//! coverage report.

use alloy_primitives::Bytes;
use alloy_sol_types::SolCall;
use ripfuzz::{
    Artifact, ArtifactId, Chain, ChainConfig, Contract, CoverageReporter, DeployInput,
    ForkDBConfig, MockTransport, Project, SharedCoverage, Transaction,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// Mock helpers
// ---------------------------------------------------------------------------

const BLOCK_NUMBER: u64 = 25_259_523;

fn block_json() -> serde_json::Value {
    json!({
        "number":"0x1816e03",
        "timestamp":"0x6a2449b7",
        "miner":"0x4838b106fce9647bdf1e7877bf73ce8b0bad5f97",
        "gasLimit":"0x392a220",
        "baseFeePerGas":"0x7d9adbf",
        "difficulty":"0x0",
        "mixHash":"0x1de43248a2093262206a08f90621f8014af9f5b1e38334591e277dcb5705b9c7",
        "excessBlobGas":"0xb1f8e90",
        "hash":"0x6bd46c9e7815ce6263a7d1c7a1237cce4303945a286e77645d51cc72c1e042de"
    })
}

alloy_sol_types::sol! {
    interface IMultiProjectHandler {
        function callAdder(address adder) external view returns (uint256);
    }
}

/// Register chain_id and block mock responses with the transport.
fn mock_fork_setup(transport: &MockTransport, url: &str) {
    let chain_id_payload = json!([{"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}]);
    let block_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[format!("0x{BLOCK_NUMBER:x}"), false]}
    ]);
    transport.mock_response(
        url,
        &chain_id_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
    );
    transport.mock_response(
        url,
        &block_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":block_json()}]),
    );
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[test]
fn external_project_coverage_report() {
    let transport = MockTransport::default();
    let url = "mock://test";

    mock_fork_setup(&transport, url);

    // Adder address: an arbitrary address we pretend is the pre-deployed adder.
    let adder_address = alloy_primitives::address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

    // Load the adder's deployed bytecode from the pre-built fixture.
    let adder_artifact_path =
        std::path::PathBuf::from("fixtures/external-coverage-adder/out/Adder.sol/Adder.json");
    let adder_artifact =
        ripfuzz::Artifact::from_json(adder_artifact_path).expect("adder artifact must parse");
    let adder_deployed_bytecode = adder_artifact
        .deployed_bytecode()
        .expect("adder must have deployed bytecode")
        .object
        .clone();

    // Mock the fork RPC responses for the adder address.
    let adder_code_result = if adder_deployed_bytecode.starts_with("0x") {
        adder_deployed_bytecode.clone()
    } else {
        format!("0x{adder_deployed_bytecode}")
    };
    let addr_hex = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let block_hex = format!("0x{BLOCK_NUMBER:x}");
    let adder_account_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":[addr_hex, block_hex]},
        {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":[addr_hex, block_hex]},
        {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":[addr_hex, block_hex]}
    ]);
    transport.mock_response(
        url,
        &adder_account_payload,
        json!([
            {"jsonrpc":"2.0","id":0,"result":"0x0"},
            {"jsonrpc":"2.0","id":1,"result":"0x1"},
            {"jsonrpc":"2.0","id":2,"result": adder_code_result}
        ]),
    );

    // Initialise fork chain.
    let config = ForkDBConfig::new(url).block_number(BLOCK_NUMBER);
    let mut chain =
        Chain::fork_with_transport(ChainConfig::default().coverage(true), config, transport)
            .expect("fork chain must init");

    // Load and deploy the handler contract.
    let handler_project = Project::new("fixtures/multi-project-coverage");
    let handler_artifacts = handler_project
        .load_artifacts()
        .expect("handler artifacts must load");
    let handler_id = ArtifactId::try_from("src/MultiProjectHandler.sol:MultiProjectHandler")
        .expect("handler artifact id must parse");
    let handler_contract =
        Contract::try_get(&handler_artifacts, &handler_id).expect("handler contract must exist");

    let deployment = chain
        .deploy(DeployInput::new(&handler_contract.initcode))
        .expect("deployment must succeed");
    assert!(deployment.result.success, "deployment must succeed");

    let target = deployment.address.expect("deployment must produce address");

    // Execute a call through the handler to the adder.
    let txs = vec![
        Transaction::new(target).calldata(Bytes::from(
            IMultiProjectHandler::callAdderCall {
                adder: adder_address,
            }
            .abi_encode(),
        )),
    ];
    let exec = chain.exec(&txs).expect("exec must succeed");
    assert_eq!(exec.results.len(), 1);
    assert!(exec.results[0].success, "callAdder must succeed");

    // Build shared coverage from deployment and execution.
    let shared = SharedCoverage::new();
    shared.merge(&deployment.coverage);
    if let Some(exec_cov) = exec.coverage {
        shared.merge(&exec_cov);
    }

    // Load artifacts from both projects (simulating --external-project).
    let mut artifacts: Vec<Artifact> = handler_artifacts.into_values().collect();
    let adder_project = Project::new("fixtures/external-coverage-adder");
    let adder_artifacts = adder_project
        .load_artifacts()
        .expect("adder artifacts must load");
    for (_, mut artifact) in adder_artifacts {
        // Use canonical form so path comparisons are robust.
        let proj_path = std::path::Path::new("fixtures/external-coverage-adder")
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from("fixtures/external-coverage-adder"));
        artifact.set_project_path(&proj_path);
        artifacts.push(artifact);
    }

    // Build the coverage report.
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/multi-project-coverage");
    let report = CoverageReporter::new()
        .build_artifacts(artifacts)
        .shared_coverage(shared)
        .base_project_path(&base)
        .build();

    // The report must include the adder's source file, qualified with
    // the external project directory since it lives outside the base project.
    let lcov = format!("{report}");
    assert!(
        lcov.contains("SF:external-coverage-adder/src/Adder.sol"),
        "lcov report must contain external-coverage-adder/src/Adder.sol, got:\n{lcov}"
    );

    // The add() function body must have line hits.
    // Line 8: `if (a > 0) {`  -- must be hit (caller passes a=1)
    // Line 9: `return a + b;` -- must be hit (branch taken)
    assert!(
        lcov.contains("DA:8,1"),
        "lcov report must have DA:8,1 (if branch hit), got:\n{lcov}"
    );
    assert!(
        lcov.contains("DA:9,1"),
        "lcov report must have DA:9,1 (return a+b hit), got:\n{lcov}"
    );

    // Line 11: `return b;` -- must NOT be hit (the else branch is not taken)
    assert!(
        !lcov.contains("DA:11,1"),
        "lcov report must NOT have DA:11,1 (else branch not taken), got:\n{lcov}"
    );

    // The report coverage percentage must be > 0 (proves lines were matched).
    assert!(
        report.coverage() > 0.0,
        "coverage percentage must be > 0, got: {:.2}%",
        report.coverage()
    );
}
