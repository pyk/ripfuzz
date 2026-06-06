//! Regression test: When chain fork mode is initialized, target contract
//! deployment for a basic contract should not trigger any RPC fetch beyond
//! the initial chain_id and block calls.

use alloy_sol_types::SolCall;
use raptor::{
    ArtifactId, Chain, ChainConfig, Contract, DeployInput, ForkDBConfig, MockTransport, Project,
    SetupInput,
};
use revm::primitives::Bytes;
use serde_json::json;

alloy_sol_types::sol! {
    interface MarketTarget {
        function touchMarket() external;
    }
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

/// Fork mode block data for mainnet block 25259523, fetched via cast block latest.
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

fn assert_no_extra_rpc(transport: &MockTransport, url: &str) {
    let chain_id_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
    ]);
    let block_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[format!("0x{BLOCK_NUMBER:x}"), false]}
    ]);
    assert_eq!(
        transport.call_count(url, &chain_id_payload),
        1,
        "must fetch chain_id exactly once"
    );
    assert_eq!(
        transport.call_count(url, &block_payload),
        1,
        "must fetch block exactly once"
    );
}

/// When chain fork mode is initialized, deploying a basic target contract
/// must not trigger any RPC fetch. Only the initial chain_id and block
/// fetches during fork initialization are expected.
#[test]
fn fork_mode_deploy_basic_contract_no_rpc_fetch() {
    let transport = MockTransport::default();
    let url = "mock://test";

    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    let config = ForkDBConfig::new(url).block_number(BLOCK_NUMBER);
    let mut chain =
        Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();

    let project = Project::new("fixtures/fork-mode-contract-creation");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/BasicContract.sol:BasicContract").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(
        deployment.result.success,
        "deployment must succeed, got: {:?}",
        deployment.result.output
    );

    assert_no_extra_rpc(&transport, url);
}

/// When chain fork mode is initialized, deploy MarketTarget, run its setup
/// (which deploys MiddleContract), and call `touchMarket` (which calls
/// MiddleContract.createLeaf, deploying LeafContract). None of this must
/// trigger any RPC fetch — nested CREATEs inside CALL execution use the
/// same ForkDB::basic_ref → None fallback.
#[test]
fn fork_mode_deploy_setup_call_no_rpc_fetch() {
    let transport = MockTransport::default();
    let url = "mock://test";

    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    let config = ForkDBConfig::new(url).block_number(BLOCK_NUMBER);
    let mut chain =
        Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();

    let project = Project::new("fixtures/fork-mode-contract-creation");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/MarketTarget.sol:MarketTarget").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    // 1. Deploy MarketTarget.
    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(deployment.result.success, "deploy must succeed");
    let target = deployment.address.unwrap();

    // 2. Setup: deploys MiddleContract.
    let setup_fn = contract
        .setup_function
        .as_ref()
        .expect("MarketTarget must have setup()");
    let setup_data = Bytes::from(setup_fn.selector().as_slice().to_vec());
    let setup_result = chain
        .setup(SetupInput::new(target).calldata(setup_data))
        .unwrap();
    assert!(setup_result.result.success, "setup must succeed");

    // 3. Call touchMarket: calls MiddleContract.createLeaf() which deploys
    //    LeafContract via a nested CREATE inside a CALL.
    let call_data = Bytes::from(MarketTarget::touchMarketCall::new(()).abi_encode());
    let call_result = chain
        .call(
            chain.deployer(),
            target,
            alloy_primitives::U256::ZERO,
            call_data,
        )
        .unwrap();
    assert!(call_result.success, "touchMarket call must succeed");

    assert_no_extra_rpc(&transport, url);
}

/// When chain fork mode is initialized, deploying a contract that creates
/// a child contract inside its `setup()` function must not trigger any RPC
/// fetch across both the deploy and the setup call.
#[test]
fn fork_mode_deploy_and_setup_factory_no_rpc_fetch() {
    let transport = MockTransport::default();
    let url = "mock://test";

    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    let config = ForkDBConfig::new(url).block_number(BLOCK_NUMBER);
    let mut chain =
        Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();

    let project = Project::new("fixtures/fork-mode-contract-creation");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/SetupFactory.sol:SetupFactory").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    // Deploy.
    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(
        deployment.result.success,
        "deployment must succeed, got: {:?}",
        deployment.result.output
    );
    let target = deployment.address.unwrap();

    // Run setup, which executes `child = new BasicContract()`.
    let setup_fn = contract
        .setup_function
        .as_ref()
        .expect("SetupFactory must have setup()");
    let setup_data = Bytes::from(setup_fn.selector().as_slice().to_vec());
    let setup_result = chain
        .setup(SetupInput::new(target).calldata(setup_data))
        .unwrap();
    assert!(
        setup_result.result.success,
        "setup must succeed, got: {:?}",
        setup_result.result.output
    );

    assert_no_extra_rpc(&transport, url);
}

/// When chain fork mode is initialized, deploying a contract that itself
/// deploys a child contract inside its constructor must not trigger any
/// RPC fetch. The nested CREATE must also hit CacheDB's fallback path
/// via ForkDB returning None, avoiding RPC entirely.
#[test]
fn fork_mode_deploy_factory_contract_no_rpc_fetch() {
    let transport = MockTransport::default();
    let url = "mock://test";

    mock_fork_setup(&transport, url, BLOCK_NUMBER, "0x1", block_json());

    let config = ForkDBConfig::new(url).block_number(BLOCK_NUMBER);
    let mut chain =
        Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();

    let project = Project::new("fixtures/fork-mode-contract-creation");
    let artifacts = project.load_artifacts().unwrap();
    let artifact_id = ArtifactId::try_from("test/FactoryContract.sol:FactoryContract").unwrap();
    let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

    let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
    assert!(
        deployment.result.success,
        "factory deployment must succeed, got: {:?}",
        deployment.result.output
    );

    assert_no_extra_rpc(&transport, url);
}
