//! Integration tests: fork mode cheatcodes.
//!
//! Verify that cheatcodes (e.g. `vm.warp`) behave correctly when the
//! EVM is running in fork mode.

use alloy_sol_types::SolCall;
use revm::primitives::Bytes;
use ripfuzz::{
    ArtifactId, Chain, ChainConfig, Contract, DeployInput, ForkDBConfig, MockTransport, Project,
    SetupInput, Transaction,
};
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

    interface VmAddr {
        function setup() external;
        function getBalance() external view returns (uint256);
    }

    interface VmDeal {
        function setup() external;
        function dealLocalAddress() external;
        function dealRemoteAddress() external;
        function getLocalBalance() external view returns (uint256);
        function getRemoteBalance() external view returns (uint256);
    }

    interface VmLoad {
        function setup() external;
        function loadLocalContract() external;
        function loadRemoteContract() external;
    }

    interface VmStore {
        function setup() external;
        function storeLocalContract() external;
        function storeRemoteContract() external;
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
/// `block.timestamp` in fork mode. The deployed harness contract is
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

/// Integration test: `vm.roll` cheatcode must correctly update
/// `block.number` in fork mode. The deployed harness contract is
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

/// Integration test: `vm.chainId` cheatcode must correctly update
/// `block.chainid` in fork mode. The deployed harness contract is
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

/// Integration test: `vm.addr` cheatcode must correctly derive a local
/// address in fork mode. The derived address is local and its balance
/// read must not trigger any RPC fetch.
#[test]
fn vm_addr() {
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
    let artifact_id = ArtifactId::try_from("test/VmAddr.sol:VmAddr").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");

    let get_balance_calldata = Bytes::from(VmAddr::getBalanceCall::new(()).abi_encode());
    let txs = [Transaction::new(target).calldata(get_balance_calldata)];

    let exec_output = chain.exec(&txs).unwrap();
    assert!(
        exec_output.results[0].success,
        "getBalance call must succeed"
    );

    let balance =
        VmAddr::getBalanceCall::abi_decode_returns(&exec_output.results[0].output.clone().unwrap())
            .unwrap();
    assert_eq!(
        balance,
        alloy_primitives::U256::ZERO,
        "derived actor balance must be zero"
    );

    assert_eq!(transport.total_calls(), 2, "no RPC calls after deployment");
}

/// Integration test: `vm.deal` cheatcode must correctly set account
/// balances in fork mode. The local address must not trigger any RPC
/// fetch, while the remote address (vitalik.eth) requires a single
/// batched account fetch.
#[test]
fn vm_deal() {
    let transport = MockTransport::default();
    let url = "mock://test";

    // Set up the vitalik account batch (balance + nonce + code) that
    // the fork DB fetches when `dealRemoteAddress` touches vitalik.eth
    // for the first time.
    let vitalik_batch_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xd8da6bf26964af9d7eed9e03e53415d37aa96045","0x1816e03"]},
        {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0xd8da6bf26964af9d7eed9e03e53415d37aa96045","0x1816e03"]},
        {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0xd8da6bf26964af9d7eed9e03e53415d37aa96045","0x1816e03"]}
    ]);
    transport.mock_response(
        url,
        &vitalik_batch_payload,
        json!([
            {"jsonrpc":"2.0","id":0,"result":"0x0"},
            {"jsonrpc":"2.0","id":1,"result":"0x0"},
            {"jsonrpc":"2.0","id":2,"result":"0x"}
        ]),
    );

    let mut chain = fork_chain(&transport, url);

    assert_eq!(
        transport.total_calls(),
        2,
        "fork init must fetch chain_id and block"
    );

    let project = Project::new("fixtures/fork-mode-cheatcode");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/VmDeal.sol:VmDeal").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");

    let deal_local_calldata = Bytes::from(VmDeal::dealLocalAddressCall::new(()).abi_encode());
    let deal_remote_calldata = Bytes::from(VmDeal::dealRemoteAddressCall::new(()).abi_encode());
    let get_local_calldata = Bytes::from(VmDeal::getLocalBalanceCall::new(()).abi_encode());
    let get_remote_calldata = Bytes::from(VmDeal::getRemoteBalanceCall::new(()).abi_encode());

    let txs = [
        Transaction::new(target).calldata(deal_local_calldata),
        Transaction::new(target).calldata(deal_remote_calldata),
        Transaction::new(target).calldata(get_local_calldata),
        Transaction::new(target).calldata(get_remote_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(
        exec_output.results[0].success,
        "dealLocalAddress must succeed"
    );
    assert!(
        exec_output.results[1].success,
        "dealRemoteAddress must succeed"
    );
    assert!(
        exec_output.results[2].success,
        "getLocalBalance must succeed"
    );
    assert!(
        exec_output.results[3].success,
        "getRemoteBalance must succeed"
    );

    let local_balance = VmDeal::getLocalBalanceCall::abi_decode_returns(
        &exec_output.results[2].output.clone().unwrap(),
    )
    .unwrap();
    let one_ether = alloy_primitives::U256::from(10_u128.pow(18));
    assert_eq!(local_balance, one_ether, "local balance must be 1 ether");

    let remote_balance = VmDeal::getRemoteBalanceCall::abi_decode_returns(
        &exec_output.results[3].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(remote_balance, one_ether, "remote balance must be 1 ether");

    assert_eq!(
        transport.total_calls(),
        3,
        "only 3 RPC calls: chain_id, block, and vitalik account batch"
    );
}

/// Integration test: `vm.load` cheatcode must correctly read storage
/// from local and remote contracts in fork mode. The local contract
/// read must not trigger any RPC fetch, while the remote (WETH) read
/// must trigger a single storage fetch.
#[test]
fn vm_load() {
    let transport = MockTransport::default();
    let url = "mock://test";

    // WETH mainnet contract at block 25_259_523: account batch
    // (balance + nonce + code) and decimals storage slot 2.
    let weth_account_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]}
    ]);
    let weth_code = include_str!("../fixtures/fork-mode-remote-address/bytecodes/weth.hex")
        .trim()
        .trim_end_matches('\n');
    transport.mock_response(
        url,
        &weth_account_payload,
        json!([
            {"jsonrpc":"2.0","id":0,"result":"0x22a0323bb2bb269993626"},
            {"jsonrpc":"2.0","id":1,"result":"0x1"},
            {"jsonrpc":"2.0","id":2,"result": weth_code}
        ]),
    );

    // WETH decimals() returns 18 (0x12) from storage slot 2.
    let decimals_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x2","0x1816e03"]}
    ]);
    transport.mock_response(
        url,
        &decimals_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":"0x0000000000000000000000000000000000000000000000000000000000000012"}]),
    );

    let mut chain = fork_chain(&transport, url);

    assert_eq!(
        transport.total_calls(),
        2,
        "fork init must fetch chain_id and block"
    );

    let project = Project::new("fixtures/fork-mode-cheatcode");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/VmLoad.sol:VmLoad").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");

    let load_local_calldata = Bytes::from(VmLoad::loadLocalContractCall::new(()).abi_encode());
    let load_remote_calldata = Bytes::from(VmLoad::loadRemoteContractCall::new(()).abi_encode());

    let txs = [
        Transaction::new(target).calldata(load_local_calldata),
        Transaction::new(target).calldata(load_remote_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(
        exec_output.results[0].success,
        "loadLocalContract must succeed"
    );
    assert!(
        exec_output.results[1].success,
        "loadRemoteContract must succeed"
    );

    assert_eq!(
        transport.total_calls(),
        4,
        "4 RPC calls: chain_id, block, WETH account batch, WETH storage fetch"
    );
}

/// Integration test: `vm.store` cheatcode must correctly write storage
/// to local and remote contracts in fork mode. The local contract
/// write must not trigger any RPC fetch, while the remote (WETH) write
/// must trigger only the basic account fetch (no storage fetch).
#[test]
fn vm_store() {
    let transport = MockTransport::default();
    let url = "mock://test";

    // WETH mainnet contract at block 25_259_523: account batch
    // (balance + nonce + code). No storage fetch needed because
    // vm.store writes directly — it never reads from the fork DB.
    let weth_account_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]}
    ]);
    let weth_code = include_str!("../fixtures/fork-mode-remote-address/bytecodes/weth.hex")
        .trim()
        .trim_end_matches('\n');
    transport.mock_response(
        url,
        &weth_account_payload,
        json!([
            {"jsonrpc":"2.0","id":0,"result":"0x22a0323bb2bb269993626"},
            {"jsonrpc":"2.0","id":1,"result":"0x1"},
            {"jsonrpc":"2.0","id":2,"result": weth_code}
        ]),
    );

    let mut chain = fork_chain(&transport, url);

    assert_eq!(
        transport.total_calls(),
        2,
        "fork init must fetch chain_id and block"
    );

    let project = Project::new("fixtures/fork-mode-cheatcode");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/VmStore.sol:VmStore").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");

    let store_local_calldata = Bytes::from(VmStore::storeLocalContractCall::new(()).abi_encode());
    let store_remote_calldata = Bytes::from(VmStore::storeRemoteContractCall::new(()).abi_encode());

    let txs = [
        Transaction::new(target).calldata(store_local_calldata),
        Transaction::new(target).calldata(store_remote_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(
        exec_output.results[0].success,
        "storeLocalContract must succeed"
    );
    assert!(
        exec_output.results[1].success,
        "storeRemoteContract must succeed"
    );

    assert_eq!(
        transport.total_calls(),
        3,
        "3 RPC calls: chain_id, block, WETH account batch (no storage fetch)"
    );
}
