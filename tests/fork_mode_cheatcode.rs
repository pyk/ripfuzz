//! Integration tests: fork mode cheatcodes.
//!
//! Verify that cheatcodes (e.g. `vm.warp`) behave correctly when the
//! EVM is running in fork mode.

use alloy_sol_types::SolCall;
use raptor::{
    ArtifactId, Chain, ChainConfig, Contract, DeployInput, ForkDBConfig, MockTransport, Project,
    Transaction,
};
use revm::primitives::Bytes;
use serde_json::json;

alloy_sol_types::sol! {
    interface VmWarp {
        function warp(uint256 value) external;
        function getBlockTimestamp() external view returns (uint256);
    }

    interface VmRoll {
        function roll(uint256 value) external;
        function getBlockNumber() external view returns (uint256);
    }

    interface VmChainId {
        function setChainId(uint256 value) external;
        function getChainId() external view returns (uint256);
    }
}

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

const BLOCK_NUMBER: u64 = 25_259_523;
/// Block timestamp at block 25_259_523 (0x6a2449b7).
const BLOCK_TIMESTAMP: u64 = 1_780_763_063;

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

fn fork_chain(transport: &MockTransport, url: &str) -> Chain {
    mock_fork_setup(transport, url, BLOCK_NUMBER, "0x1", block_json());
    let config = ForkDBConfig::new(url).block_number(BLOCK_NUMBER);
    Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap()
}

/// Integration test: `vm.warp` cheatcode must correctly update
/// `block.timestamp` in fork mode. The deployed target contract is
/// local and must not trigger any RPC fetch.
#[test]
fn vm_warp() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = fork_chain(&transport, url);

    assert_eq!(
        transport.total_calls(),
        2,
        "fork init must fetch chain_id and block"
    );

    let project = Project::new("fixtures/fork-mode-cheatcode");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/VmWarp.sol:VmWarp").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let warp_value = alloy_primitives::U256::from(42);
    let warp_calldata = Bytes::from(VmWarp::warpCall::new((warp_value,)).abi_encode());
    let get_timestamp_calldata = Bytes::from(VmWarp::getBlockTimestampCall::new(()).abi_encode());

    let txs = [
        Transaction::new(target).calldata(warp_calldata),
        Transaction::new(target).calldata(get_timestamp_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(exec_output.results[0].success, "warp call must succeed");
    assert!(
        exec_output.results[1].success,
        "getBlockTimestamp call must succeed"
    );

    let ts = VmWarp::getBlockTimestampCall::abi_decode_returns(
        &exec_output.results[1].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        ts,
        alloy_primitives::U256::from(BLOCK_TIMESTAMP + 42),
        "block timestamp must equal fork timestamp + warp value"
    );

    assert_eq!(transport.total_calls(), 2, "no RPC calls after deployment");
}

/// Integration test: `vm.chainId` cheatcode must correctly update
/// `block.chainid` in fork mode. The deployed target contract is
/// local and must not trigger any RPC fetch.
#[test]
fn vm_chain_id() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = fork_chain(&transport, url);

    assert_eq!(
        transport.total_calls(),
        2,
        "fork init must fetch chain_id and block"
    );

    let project = Project::new("fixtures/fork-mode-cheatcode");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/VmChainId.sol:VmChainId").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let chain_id_value = alloy_primitives::U256::from(1337);
    let set_chain_id_calldata =
        Bytes::from(VmChainId::setChainIdCall::new((chain_id_value,)).abi_encode());
    let get_chain_id_calldata = Bytes::from(VmChainId::getChainIdCall::new(()).abi_encode());

    let txs = [
        Transaction::new(target).calldata(set_chain_id_calldata),
        Transaction::new(target).calldata(get_chain_id_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(
        exec_output.results[0].success,
        "setChainId call must succeed"
    );
    assert!(
        exec_output.results[1].success,
        "getChainId call must succeed"
    );

    let id = VmChainId::getChainIdCall::abi_decode_returns(
        &exec_output.results[1].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        id, chain_id_value,
        "chain ID must equal the value set by vm.chainId"
    );

    assert_eq!(transport.total_calls(), 2, "no RPC calls after deployment");
}

/// Integration test: `vm.roll` cheatcode must correctly update
/// `block.number` in fork mode. The deployed target contract is
/// local and must not trigger any RPC fetch.
#[test]
fn vm_roll() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = fork_chain(&transport, url);

    assert_eq!(
        transport.total_calls(),
        2,
        "fork init must fetch chain_id and block"
    );

    let project = Project::new("fixtures/fork-mode-cheatcode");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/VmRoll.sol:VmRoll").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let roll_value = alloy_primitives::U256::from(42);
    let roll_calldata = Bytes::from(VmRoll::rollCall::new((roll_value,)).abi_encode());
    let get_block_calldata = Bytes::from(VmRoll::getBlockNumberCall::new(()).abi_encode());

    let txs = [
        Transaction::new(target).calldata(roll_calldata),
        Transaction::new(target).calldata(get_block_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(exec_output.results[0].success, "roll call must succeed");
    assert!(
        exec_output.results[1].success,
        "getBlockNumber call must succeed"
    );

    let bn = VmRoll::getBlockNumberCall::abi_decode_returns(
        &exec_output.results[1].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        bn,
        alloy_primitives::U256::from(BLOCK_NUMBER + 42),
        "block number must equal fork block number + roll value"
    );

    assert_eq!(transport.total_calls(), 2, "no RPC calls after deployment");
}
