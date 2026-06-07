//! Regression test: fork mode remote address.
//!
//! When chain fork mode is initialized, a target contract whose constructor
//! reads on-chain state (e.g. vitalik.eth balance) must receive real fork
//! data via the normal ForkDB lazy-fetch path.

use raptor::{
    ArtifactId, Chain, ChainConfig, Contract, DeployInput, ForkDBConfig, MockTransport, Project,
};
use serde_json::json;

/// Register chain_id and block mock responses with the transport.
fn mock_fork_setup(
    transport: &MockTransport,
    url: &str,
    block_number: u64,
    chain_id_hex: &str,
    block_json: serde_json::Value,
) {
    let chain_id_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
    ]);
    let block_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[format!("0x{block_number:x}"), false]}
    ]);
    transport.mock_response(
        url,
        &chain_id_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":chain_id_hex}]),
    );
    transport.mock_response(
        url,
        &block_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":block_json}]),
    );
}

/// Fork mode block data for mainnet block 25259523.
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

/// When chain fork mode is initialized, deploying a contract whose
/// constructor reads vitalik.eth balance must receive the real on-chain
/// balance via the normal ForkDB RPC path.
#[test]
fn fork_mode_constructor_reads_vitalik_balance() {
    let transport = MockTransport::default();
    let url = "mock://test";

    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    // vitalik.eth — real on-chain account with balance. The contract's
    // constructor reads vitalik's balance, which triggers a normal ForkDB
    // RPC fetch. The CREATE address and coinbase are handled internally
    // (LocalTracker marks CREATE addresses as local; coinbase is seeded
    // during fork init) so they do not need mock responses.
    let vitalik_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xd8da6bf26964af9d7eed9e03e53415d37aa96045","0x1816e03"]},
        {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0xd8da6bf26964af9d7eed9e03e53415d37aa96045","0x1816e03"]},
        {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0xd8da6bf26964af9d7eed9e03e53415d37aa96045","0x1816e03"]}
    ]);
    // 0x4ef07de0a9f8eaa6 = 5_688_184_733_246_745_254 wei
    transport.mock_response(
        url,
        &vitalik_payload,
        json!([
            {"jsonrpc":"2.0","id":0,"result":"0x4ef07de0a9f8eaa6"},
            {"jsonrpc":"2.0","id":1,"result":"0x0"},
            {"jsonrpc":"2.0","id":2,"result":"0x"}
        ]),
    );

    let config = ForkDBConfig::new(url).block_number(BLOCK_NUMBER);
    let mut chain =
        Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();

    let project = Project::new("fixtures/fork-mode-remote-address");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/VitalikBalance.sol:VitalikBalance").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
}
