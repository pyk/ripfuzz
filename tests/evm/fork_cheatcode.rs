//! Integration tests: fork mode cheatcodes.
//!
//! Verify that cheatcodes (e.g. `rvm.warp`) behave correctly when the
//! harness enters fork mode via `rvm.fork`.

use std::sync::Arc;

use alloy_sol_types::SolCall;
use revm::primitives::Bytes;
use ripfuzz::compilers::solc::{Solc, SolcOutput};
use ripfuzz::evm::{
    Chain, ChainConfig, DeployInput, ForkDBConfig, MockTransport, SetupInput, Transaction,
};
use ripfuzz::harness::HarnessId;
use serde_json::json;

alloy_sol_types::sol! {
    interface RvmWarp {
        function setup() external;
        function warp(uint256 value) external;
        function getBlockTimestamp() external view returns (uint256);
    }

    interface RvmRoll {
        function setup() external;
        function roll(uint256 value) external;
        function getBlockNumber() external view returns (uint256);
    }

    interface RvmChainId {
        function setup() external;
        function setChainId(uint256 value) external;
        function getChainId() external view returns (uint256);
    }

    interface RvmAddr {
        function setup() external;
        function getBalance() external view returns (uint256);
    }

    interface RvmDeal {
        function setup() external;
        function dealLocalAddress() external;
        function dealRemoteAddress() external;
        function getLocalBalance() external view returns (uint256);
        function getRemoteBalance() external view returns (uint256);
    }

    interface RvmLoad {
        function setup() external;
        function loadLocalContract() external;
        function loadRemoteContract() external;
    }

    interface RvmStore {
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

fn empty_chain(transport: MockTransport) -> Chain {
    let config = ChainConfig::default()
        .with_transport(Arc::new(transport))
        .with_fork_defaults(ForkDBConfig::new(""));
    Chain::new(config).unwrap()
}

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

fn load_initcode(root: &str, target: &str) -> String {
    compile_fixture(root, target).initcode().unwrap().to_owned()
}

fn load_contract(id: &str) -> String {
    load_initcode("fixtures/evm/fork-cheatcode", id)
}

/// Integration test: `rvm.warp` cheatcode must correctly update
/// `block.timestamp` in fork mode. The deployed harness contract is
/// local and must not trigger any RPC fetch beyond fork init.
#[test]
fn rvm_warp() {
    let transport = MockTransport::default();
    let url = "mock://test";
    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    let mut chain = empty_chain(transport.clone());
    assert_eq!(
        transport.total_calls(),
        0,
        "no RPC calls before harness rvm.fork"
    );

    let initcode = load_contract("RvmWarp.sol:RvmWarp");
    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup.result.success, "setup (rvm.fork) must succeed");
    assert_eq!(
        transport.total_calls(),
        2,
        "setup fork must fetch chain_id and block"
    );

    let warp_value = alloy_primitives::U256::from(42);
    let warp_calldata = Bytes::from(RvmWarp::warpCall::new((warp_value,)).abi_encode());
    let get_timestamp_calldata = Bytes::from(RvmWarp::getBlockTimestampCall::new(()).abi_encode());

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

    let ts = RvmWarp::getBlockTimestampCall::abi_decode_returns(
        &exec_output.results[1].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        ts,
        alloy_primitives::U256::from(BLOCK_TIMESTAMP + 42),
        "block timestamp must equal fork timestamp + warp value"
    );

    assert_eq!(transport.total_calls(), 2, "no RPC calls after setup");
}

/// Integration test: `rvm.roll` cheatcode must correctly update
/// `block.number` in fork mode.
#[test]
fn rvm_roll() {
    let transport = MockTransport::default();
    let url = "mock://test";
    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    let mut chain = empty_chain(transport.clone());
    let initcode = load_contract("RvmRoll.sol:RvmRoll");

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup.result.success, "setup (rvm.fork) must succeed");
    assert_eq!(
        transport.total_calls(),
        2,
        "setup fork must fetch chain_id and block"
    );

    let roll_value = alloy_primitives::U256::from(42);
    let roll_calldata = Bytes::from(RvmRoll::rollCall::new((roll_value,)).abi_encode());
    let get_block_calldata = Bytes::from(RvmRoll::getBlockNumberCall::new(()).abi_encode());

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

    let bn = RvmRoll::getBlockNumberCall::abi_decode_returns(
        &exec_output.results[1].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        bn,
        alloy_primitives::U256::from(BLOCK_NUMBER + 42),
        "block number must equal fork block number + roll value"
    );

    assert_eq!(transport.total_calls(), 2, "no RPC calls after setup");
}

/// Integration test: `rvm.chainId` cheatcode must correctly update
/// `block.chainid` in fork mode.
#[test]
fn rvm_chain_id() {
    let transport = MockTransport::default();
    let url = "mock://test";
    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    let mut chain = empty_chain(transport.clone());
    let initcode = load_contract("RvmChainId.sol:RvmChainId");

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup.result.success, "setup (rvm.fork) must succeed");
    assert_eq!(
        transport.total_calls(),
        2,
        "setup fork must fetch chain_id and block"
    );

    let chain_id_value = alloy_primitives::U256::from(1337);
    let set_chain_id_calldata =
        Bytes::from(RvmChainId::setChainIdCall::new((chain_id_value,)).abi_encode());
    let get_chain_id_calldata = Bytes::from(RvmChainId::getChainIdCall::new(()).abi_encode());

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

    let id = RvmChainId::getChainIdCall::abi_decode_returns(
        &exec_output.results[1].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        id, chain_id_value,
        "chain ID must equal the value set by rvm.chainId"
    );

    assert_eq!(transport.total_calls(), 2, "no RPC calls after setup");
}

/// Integration test: `rvm.addr` cheatcode must correctly derive a local
/// address in fork mode. The derived address is local and its balance
/// read must not trigger any RPC fetch.
#[test]
fn rvm_addr() {
    let transport = MockTransport::default();
    let url = "mock://test";
    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    let mut chain = empty_chain(transport.clone());
    let initcode = load_contract("RvmAddr.sol:RvmAddr");

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");
    assert_eq!(
        transport.total_calls(),
        2,
        "setup fork must fetch chain_id and block"
    );

    let get_balance_calldata = Bytes::from(RvmAddr::getBalanceCall::new(()).abi_encode());
    let txs = [Transaction::new(target).calldata(get_balance_calldata)];

    let exec_output = chain.exec(&txs).unwrap();
    assert!(
        exec_output.results[0].success,
        "getBalance call must succeed"
    );

    let balance = RvmAddr::getBalanceCall::abi_decode_returns(
        &exec_output.results[0].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        balance,
        alloy_primitives::U256::ZERO,
        "derived actor balance must be zero"
    );

    assert_eq!(transport.total_calls(), 2, "no RPC calls after setup");
}

/// Integration test: `rvm.deal` cheatcode must correctly set account
/// balances in fork mode. The local address must not trigger any RPC
/// fetch, while the remote address (vitalik.eth) requires a single
/// batched account fetch.
#[test]
fn rvm_deal() {
    let transport = MockTransport::default();
    let url = "mock://test";
    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

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

    let mut chain = empty_chain(transport.clone());
    let initcode = load_contract("RvmDeal.sol:RvmDeal");

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");
    assert_eq!(
        transport.total_calls(),
        2,
        "setup fork must fetch chain_id and block"
    );

    let deal_local_calldata = Bytes::from(RvmDeal::dealLocalAddressCall::new(()).abi_encode());
    let deal_remote_calldata = Bytes::from(RvmDeal::dealRemoteAddressCall::new(()).abi_encode());
    let get_local_calldata = Bytes::from(RvmDeal::getLocalBalanceCall::new(()).abi_encode());
    let get_remote_calldata = Bytes::from(RvmDeal::getRemoteBalanceCall::new(()).abi_encode());

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

    let local_balance = RvmDeal::getLocalBalanceCall::abi_decode_returns(
        &exec_output.results[2].output.clone().unwrap(),
    )
    .unwrap();
    let one_ether = alloy_primitives::U256::from(10_u128.pow(18));
    assert_eq!(local_balance, one_ether, "local balance must be 1 ether");

    let remote_balance = RvmDeal::getRemoteBalanceCall::abi_decode_returns(
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

/// Integration test: `rvm.load` cheatcode must correctly read storage
/// from local and remote contracts in fork mode.
#[test]
fn rvm_load() {
    let transport = MockTransport::default();
    let url = "mock://test";
    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    // WETH mainnet contract at block 25_259_523: account batch
    // (balance + nonce + code) and decimals storage slot 2.
    let weth_account_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]}
    ]);
    let weth_code = include_str!("../../fixtures/evm/fork-remote-address/bytecodes/weth.hex")
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

    let mut chain = empty_chain(transport.clone());
    let initcode = load_contract("RvmLoad.sol:RvmLoad");

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");
    assert_eq!(
        transport.total_calls(),
        2,
        "setup fork must fetch chain_id and block"
    );

    let load_local_calldata = Bytes::from(RvmLoad::loadLocalContractCall::new(()).abi_encode());
    let load_remote_calldata = Bytes::from(RvmLoad::loadRemoteContractCall::new(()).abi_encode());

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

/// Integration test: `rvm.store` cheatcode must correctly write storage
/// to local and remote contracts in fork mode.
#[test]
fn rvm_store() {
    let transport = MockTransport::default();
    let url = "mock://test";
    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    // WETH mainnet contract at block 25_259_523: account batch
    // (balance + nonce + code). No storage fetch needed because
    // rvm.store writes directly. It never reads from the fork DB.
    let weth_account_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]}
    ]);
    let weth_code = include_str!("../../fixtures/evm/fork-remote-address/bytecodes/weth.hex")
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

    let mut chain = empty_chain(transport.clone());
    let initcode = load_contract("RvmStore.sol:RvmStore");

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");
    assert_eq!(
        transport.total_calls(),
        2,
        "setup fork must fetch chain_id and block"
    );

    let store_local_calldata = Bytes::from(RvmStore::storeLocalContractCall::new(()).abi_encode());
    let store_remote_calldata =
        Bytes::from(RvmStore::storeRemoteContractCall::new(()).abi_encode());

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
        "3 RPC calls: chain_id, block, and WETH account batch"
    );
}
