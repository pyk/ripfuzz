//! Regression tests: fork mode contract deployment.
//!
//! When the harness enters fork mode via `rvm.fork`, the Chain is aware of all
//! addresses created inside the harness contract and skips remote RPC
//! fetches for them. Only genuine on-chain accounts trigger RPC.

use std::sync::Arc;

use alloy_sol_types::SolCall;
use revm::primitives::Bytes;
use ripfuzz::evm::{
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

alloy_sol_types::sol! {
    interface BasicContract {
        function setValue(uint256 newValue) external;
        function value() external view returns (uint256);
    }

    interface DeployChildInConstructor {
        function setChildValue(uint256 newValue) external;
        function invariant_child_exists() external;
        function child() external view returns (address);
    }

    interface DeployChildInSetup {
        function setChildValue(uint256 newValue) external;
        function invariant_child_exists() external;
        function child() external view returns (address);
    }


    interface DeployChildInHandlerFunction {
        function createMarket() external;
        function checkMarket() external;
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

fn empty_chain(transport: &MockTransport, url: &str) -> Chain {
    mock_fork_setup(transport, url, BLOCK_NUMBER, "0x1", block_json());
    let config = ChainConfig::default()
        .with_transport(Arc::new(transport.clone()))
        .with_fork_defaults(ForkDBConfig::new(""));
    Chain::new(config).unwrap()
}

/// Regression test: basic contract deployment must not trigger an RPC
/// fetch for the newly created address. Interacting with the deployed
/// contract via `exec` must also not trigger any RPC.
#[test]
fn deploy_basic_contract() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = empty_chain(&transport, url);

    let initcode = load_initcode(
        "fixtures/evm/fork-contract-deployment",
        "BasicContract.sol:BasicContract",
    );

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    // Constructor called rvm.fork: chain_id + block only.
    let baseline = transport.total_calls();
    assert_eq!(
        baseline, 2,
        "constructor fork must fetch chain_id and block"
    );

    let set_value_calldata = Bytes::from(
        BasicContract::setValueCall::new((alloy_primitives::U256::from(7),)).abi_encode(),
    );
    let value_calldata = Bytes::from(BasicContract::valueCall::new(()).abi_encode());
    let txs = [
        Transaction::new(target).calldata(set_value_calldata),
        Transaction::new(target).calldata(value_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(exec_output.results[0].success, "setValue call must succeed");
    assert!(exec_output.results[1].success, "value call must succeed");
    let stored = BasicContract::valueCall::abi_decode_returns(
        &exec_output.results[1].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        stored,
        alloy_primitives::U256::from(7),
        "stored value must be 7"
    );

    assert_eq!(
        transport.total_calls(),
        baseline,
        "no RPC calls after deployment"
    );
}

/// Regression test: deploying a child contract inside a constructor must
/// not trigger an RPC fetch for the child's address. Interacting with the
/// deployed contract (calling into the child) must also not trigger any RPC.
#[test]
fn deploy_child_in_constructor() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = empty_chain(&transport, url);

    let initcode = load_initcode(
        "fixtures/evm/fork-contract-deployment",
        "DeployChildInConstructor.sol:DeployChildInConstructor",
    );

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    // Parent constructor forks, then child constructor re-selects the same fork.
    let baseline = transport.total_calls();
    assert_eq!(
        baseline, 2,
        "constructor fork must fetch chain_id and block once"
    );

    let set_child_calldata = Bytes::from(
        DeployChildInConstructor::setChildValueCall::new((alloy_primitives::U256::from(7),))
            .abi_encode(),
    );
    let child_calldata = Bytes::from(DeployChildInConstructor::childCall::new(()).abi_encode());
    let txs = [
        Transaction::new(target).calldata(set_child_calldata),
        Transaction::new(target).calldata(child_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(
        exec_output.results[0].success,
        "setChildValue call must succeed"
    );
    assert!(exec_output.results[1].success, "child() call must succeed");
    let child_addr = DeployChildInConstructor::childCall::abi_decode_returns(
        &exec_output.results[1].output.clone().unwrap(),
    )
    .unwrap();
    assert_ne!(
        child_addr,
        alloy_primitives::Address::ZERO,
        "child must be non-zero address"
    );

    // Read back the child's value to confirm setChildValue took effect.
    let value_calldata = Bytes::from(BasicContract::valueCall::new(()).abi_encode());
    let value_tx = Transaction::new(child_addr).calldata(value_calldata);
    let exec_output = chain.exec(&[value_tx]).unwrap();
    assert!(
        exec_output.results[0].success,
        "child value() call must succeed"
    );
    let stored = BasicContract::valueCall::abi_decode_returns(
        &exec_output.results[0].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        stored,
        alloy_primitives::U256::from(7),
        "child stored value must be 7"
    );

    assert_eq!(
        transport.total_calls(),
        baseline,
        "no RPC calls after deployment"
    );
}

/// Regression test: deploying a child contract inside a setup function
/// must not trigger an RPC fetch for the child's address. Interacting
/// with the deployed contract (calling into the child) must also not
/// trigger any RPC.
#[test]
fn deploy_child_in_setup() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = empty_chain(&transport, url);

    let initcode = load_initcode(
        "fixtures/evm/fork-contract-deployment",
        "DeployChildInSetup.sol:DeployChildInSetup",
    );

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    assert_eq!(
        transport.total_calls(),
        0,
        "deploy on empty sandbox must not fetch remote state"
    );

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");

    let baseline = transport.total_calls();
    assert_eq!(baseline, 2, "setup fork must fetch chain_id and block once");

    let set_child_calldata = Bytes::from(
        DeployChildInSetup::setChildValueCall::new((alloy_primitives::U256::from(7),)).abi_encode(),
    );
    let child_calldata = Bytes::from(DeployChildInSetup::childCall::new(()).abi_encode());
    let txs = [
        Transaction::new(target).calldata(set_child_calldata),
        Transaction::new(target).calldata(child_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(
        exec_output.results[0].success,
        "setChildValue call must succeed"
    );
    assert!(exec_output.results[1].success, "child() call must succeed");
    let child_addr = DeployChildInSetup::childCall::abi_decode_returns(
        &exec_output.results[1].output.clone().unwrap(),
    )
    .unwrap();
    assert_ne!(
        child_addr,
        alloy_primitives::Address::ZERO,
        "child must be non-zero address"
    );

    // Read back the child's value to confirm setChildValue took effect.
    let value_calldata = Bytes::from(BasicContract::valueCall::new(()).abi_encode());
    let value_tx = Transaction::new(child_addr).calldata(value_calldata);
    let exec_output = chain.exec(&[value_tx]).unwrap();
    assert!(
        exec_output.results[0].success,
        "child value() call must succeed"
    );
    let stored = BasicContract::valueCall::abi_decode_returns(
        &exec_output.results[0].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(
        stored,
        alloy_primitives::U256::from(7),
        "child stored value must be 7"
    );

    assert_eq!(
        transport.total_calls(),
        baseline,
        "no RPC calls after setup and deployment"
    );
}

/// Regression test: deploying a child contract inside a handler function
/// must not trigger an RPC fetch for the child's address. Interacting
/// with the created child in a subsequent handler function must also not
/// trigger any RPC.
#[test]
fn deploy_child_in_handler_function() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = empty_chain(&transport, url);

    let initcode = load_initcode(
        "fixtures/evm/fork-contract-deployment",
        "DeployChildInHandlerFunction.sol:DeployChildInHandlerFunction",
    );

    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_result = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup_result.result.success, "setup must succeed");

    let baseline = transport.total_calls();
    assert_eq!(baseline, 2, "setup fork must fetch chain_id and block once");

    let create_market_calldata =
        Bytes::from(DeployChildInHandlerFunction::createMarketCall::new(()).abi_encode());
    let check_market_calldata =
        Bytes::from(DeployChildInHandlerFunction::checkMarketCall::new(()).abi_encode());
    let txs = [
        Transaction::new(target).calldata(create_market_calldata),
        Transaction::new(target).calldata(check_market_calldata),
    ];
    let exec_output = chain.exec(&txs).unwrap();
    assert!(
        exec_output.results[0].success,
        "createMarket call must succeed"
    );
    assert!(
        exec_output.results[1].success,
        "checkMarket call must succeed"
    );

    assert_eq!(
        transport.total_calls(),
        baseline,
        "no RPC calls after setup"
    );
}
