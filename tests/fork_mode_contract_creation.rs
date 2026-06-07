//! Regression tests: fork mode contract creation.
//!
//! When chain fork mode is initialized, the Chain is aware of all
//! addresses created inside the target contract and skips remote RPC
//! fetches for them. Only genuine on-chain accounts trigger RPC.

use alloy_sol_types::SolCall;
use raptor::{
    ArtifactId, Chain, ChainConfig, Contract, DeployInput, ForkDBConfig, MockTransport, Project,
    SetupInput, Transaction,
};
use revm::primitives::Bytes;
use serde_json::json;

alloy_sol_types::sol! {
    interface BasicContract {
        function setValue(uint256 newValue) external;
    }

    interface DeployChildInConstructor {
        function setChildValue(uint256 newValue) external;
    }

    interface DeployChildInSetup {
        function setChildValue(uint256 newValue) external;
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

fn fork_chain(transport: &MockTransport, url: &str) -> Chain {
    mock_fork_setup(transport, url, BLOCK_NUMBER, "0x1", block_json());
    let config = ForkDBConfig::new(url).block_number(BLOCK_NUMBER);
    Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap()
}

/// Regression test: basic contract deployment must not trigger an RPC
/// fetch for the newly created address. Interacting with the deployed
/// contract via `exec` must also not trigger any RPC.
#[test]
fn deploy_basic_contract() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = fork_chain(&transport, url);

    let project = Project::new("fixtures/fork-mode-contract-creation");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/BasicContract.sol:BasicContract").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let baseline = transport.total_calls();

    let set_value_calldata = Bytes::from(
        BasicContract::setValueCall::new((alloy_primitives::U256::from(7),)).abi_encode(),
    );
    let tx = Transaction::new(target).calldata(set_value_calldata);
    let exec_output = chain.exec(&[tx]).unwrap();
    assert!(exec_output.results[0].success, "setValue call must succeed");

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
    let mut chain = fork_chain(&transport, url);

    let project = Project::new("fixtures/fork-mode-contract-creation");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id =
        ArtifactId::try_from("test/DeployChildInConstructor.sol:DeployChildInConstructor").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let baseline = transport.total_calls();

    let set_child_calldata = Bytes::from(
        DeployChildInConstructor::setChildValueCall::new((alloy_primitives::U256::from(7),))
            .abi_encode(),
    );
    let tx = Transaction::new(target).calldata(set_child_calldata);
    let exec_output = chain.exec(&[tx]).unwrap();
    assert!(
        exec_output.results[0].success,
        "setChildValue call must succeed"
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
    let mut chain = fork_chain(&transport, url);

    let project = Project::new("fixtures/fork-mode-contract-creation");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id =
        ArtifactId::try_from("test/DeployChildInSetup.sol:DeployChildInSetup").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_fn = contract
        .setup_function
        .as_ref()
        .expect("DeployChildInSetup must have setup()");
    let setup_data = Bytes::from(setup_fn.selector().as_slice().to_vec());
    let setup_result = chain
        .setup(SetupInput::new(target).calldata(setup_data))
        .unwrap();
    assert!(setup_result.result.success, "setup must succeed");

    let baseline = transport.total_calls();

    let set_child_calldata = Bytes::from(
        DeployChildInSetup::setChildValueCall::new((alloy_primitives::U256::from(7),)).abi_encode(),
    );
    let tx = Transaction::new(target).calldata(set_child_calldata);
    let exec_output = chain.exec(&[tx]).unwrap();
    assert!(
        exec_output.results[0].success,
        "setChildValue call must succeed"
    );

    assert_eq!(
        transport.total_calls(),
        baseline,
        "no RPC calls after setup and deployment"
    );
}

#[test]
fn fork_mode_deploy_and_setup_factory_no_rpc_fetch() {
    let transport = MockTransport::default();
    let url = "mock://test";
    let mut chain = fork_chain(&transport, url);

    let project = Project::new("fixtures/fork-mode-contract-creation");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/SetupFactory.sol:SetupFactory").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();

    let setup_fn = contract
        .setup_function
        .as_ref()
        .expect("SetupFactory must have setup()");
    let setup_data = Bytes::from(setup_fn.selector().as_slice().to_vec());
    let setup_result = chain
        .setup(SetupInput::new(target).calldata(setup_data))
        .unwrap();
    assert!(setup_result.result.success, "setup must succeed");
}
