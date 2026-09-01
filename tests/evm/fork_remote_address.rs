//! Regression test: fork mode remote address.
//!
//! When the harness calls `rvm.fork`, a contract that reads on-chain state
//! (e.g. vitalik.eth balance) must receive real fork data via the normal
//! ForkDB lazy-fetch path and cache the result so that subsequent reads across
//! constructor, setup, handler function, and invariant function do not trigger
//! additional RPC calls.

use std::sync::Arc;

use alloy_sol_types::SolCall;
use ripfuzz::{
    Chain, ChainConfig, DeployInput, ForkDBConfig, MockTransport, SetupInput, Transaction,
};
use serde_json::json;

use ripfuzz::compilers::solc::{Solc, SolcOutput};
use ripfuzz::harness::HarnessId;

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

fn empty_chain(transport: MockTransport) -> Chain {
    let config = ChainConfig::default()
        .with_transport(Arc::new(transport))
        .with_fork_defaults(ForkDBConfig::new(""));
    Chain::new(config).unwrap()
}

alloy_sol_types::sol! {
    interface RemoteAccountBalance {
        function setup() external;
        function checkBalance() external;
        function invariant_checkBalance() external view;
    }

    interface InteractWithWETH {
        function setup() external;
        function checkBalance() external;
        function invariant_checkBalance() external view;
    }
}

/// Regression test: deploying and interacting with a contract whose
/// constructor, setup, handler function, and invariant function all read
/// vitalik.eth balance must fetch the remote account exactly once and
/// cache the result for all subsequent reads.
#[test]
fn remote_account_balance() {
    let transport = MockTransport::default();
    let url = "mock://test";

    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    assert_eq!(
        transport.total_calls(),
        0,
        "no calls before harness rvm.fork"
    );

    // vitalik.eth - real on-chain account with balance. The constructor
    // forks then reads vitalik's balance, which triggers a normal ForkDB RPC
    // fetch. The CREATE address and coinbase are handled internally
    // (LocalTracker marks CREATE addresses as local; coinbase is seeded
    // during fork init) so they do not need mock responses.
    let vitalik_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xd8da6bf26964af9d7eed9e03e53415d37aa96045","0x1816e03"]},
        {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0xd8da6bf26964af9d7eed9e03e53415d37aa96045","0x1816e03"]},
        {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0xd8da6bf26964af9d7eed9e03e53415d37aa96045","0x1816e03"]}
    ]);
    // 0x4ef0b08c7783eea6 = 5_688_240_446_715_981_478 wei
    transport.mock_response(
        url,
        &vitalik_payload,
        json!([
            {"jsonrpc":"2.0","id":0,"result":"0x4ef0b08c7783eea6"},
            {"jsonrpc":"2.0","id":1,"result":"0x0"},
            {"jsonrpc":"2.0","id":2,"result":"0x"}
        ]),
    );

    let mut chain = empty_chain(transport.clone());

    let initcode = load_initcode(
        "fixtures/evm/fork-remote-address",
        "RemoteAccountBalance.sol:RemoteAccountBalance",
    );

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");

    // Deployment constructor: fork init (chain_id + block) + vitalik account batch.
    assert_eq!(
        transport.total_calls(),
        3,
        "3 total calls after deployment: chain_id, block, and vitalik account"
    );

    let target = deployment.address.unwrap();

    // setup reads vitalik balance (cached, no new RPC).
    let setup_output = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_output.result.success, "setup must succeed");

    // checkBalance handler function and invariant_checkBalance invocation.
    let txs = vec![
        Transaction::new(target).calldata(
            RemoteAccountBalance::checkBalanceCall::new(())
                .abi_encode()
                .into(),
        ),
        Transaction::new(target).calldata(
            RemoteAccountBalance::invariant_checkBalanceCall::new(())
                .abi_encode()
                .into(),
        ),
    ];

    let exec_output = chain.exec(&txs).unwrap();
    assert_eq!(exec_output.results.len(), 2);
    assert!(
        exec_output.results[0].success,
        "checkBalance call must succeed"
    );
    assert!(
        exec_output.results[1].success,
        "invariant_checkBalance call must succeed"
    );

    // All subsequent vitalik balance reads are cached, no new RPC calls.
    assert_eq!(
        transport.total_calls(),
        3,
        "total calls must remain 3 after cached reads"
    );
}

/// Regression test: interacting with mainnet WETH contract in fork mode must
/// fetch the remote contract account and storage slots exactly once and cache
/// the results for all subsequent reads across constructor, setup, target
/// function, and invariant function.
#[test]
fn interact_with_weth() {
    let transport = MockTransport::default();
    let url = "mock://test";

    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    assert_eq!(
        transport.total_calls(),
        0,
        "no calls before harness rvm.fork"
    );

    // WETH mainnet contract account data at block 25_259_523.
    let weth_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]},
        {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x1816e03"]}
    ]);
    let weth_code = include_str!("../../fixtures/evm/fork-remote-address/bytecodes/weth.hex")
        .trim()
        .trim_end_matches('\n');
    transport.mock_response(
        url,
        &weth_payload,
        json!([
            {"jsonrpc":"2.0","id":0,"result":"0x22a0323bb2bb269993626"},
            {"jsonrpc":"2.0","id":1,"result":"0x1"},
            {"jsonrpc":"2.0","id":2,"result": weth_code}
        ]),
    );

    // WETH decimals() reads from storage slot 2 (returns 0x12 = 18).
    let decimals_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x2","0x1816e03"]}
    ]);
    transport.mock_response(
        url,
        &decimals_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":"0x0000000000000000000000000000000000000000000000000000000000000012"}]),
    );

    // WETH balanceOf(vitalik) storage slot.
    let balance_slot_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getStorageAt","params":["0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","0x3a988d762a24303c37d08f1543db6143453b579691d5c20fed39629ff1334cca","0x1816e03"]}
    ]);
    transport.mock_response(
        url,
        &balance_slot_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":"0x0000000000000000000000000000000000000000000000001449b4a27c274de6"}]),
    );

    let mut chain = empty_chain(transport.clone());

    let initcode = load_initcode(
        "fixtures/evm/fork-remote-address",
        "InteractWithWETH.sol:InteractWithWETH",
    );

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");

    // Deployment constructor: fork init + WETH account + decimals storage.
    assert_eq!(
        transport.total_calls(),
        4,
        "4 total calls after deployment: chain_id, block, WETH account, decimals storage"
    );

    let target = deployment.address.unwrap();

    // setup reads WETH balanceOf(vitalik) (balanceOf storage slot, not yet cached).
    let setup_output = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_output.result.success, "setup must succeed");

    // checkBalance and invariant_checkBalance (all cached, no new RPC).
    let txs = vec![
        Transaction::new(target).calldata(
            InteractWithWETH::checkBalanceCall::new(())
                .abi_encode()
                .into(),
        ),
        Transaction::new(target).calldata(
            InteractWithWETH::invariant_checkBalanceCall::new(())
                .abi_encode()
                .into(),
        ),
    ];

    let exec_output = chain.exec(&txs).unwrap();
    assert_eq!(exec_output.results.len(), 2);
    assert!(
        exec_output.results[0].success,
        "checkBalance call must succeed"
    );
    assert!(
        exec_output.results[1].success,
        "invariant_checkBalance call must succeed"
    );

    // balanceOf storage read during setup adds 1 call; everything cached after that.
    assert_eq!(
        transport.total_calls(),
        5,
        "total calls: chain_id, block, WETH account, decimals storage, balanceOf storage"
    );
}
