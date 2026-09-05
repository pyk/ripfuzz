//! Integration test: multiple forks in one call sequence.
//!
//! Mirrors a fuzzer corpus item: several handler calls run in a single
//! `chain.exec(&[...])`. Covers:
//! - remote state isolation (same address on eth vs polygon)
//! - remote mutation persistence when switching back
//! - local harness storage surviving fork switches (value conservation / ghost
//!   accounting across chains)

use std::sync::Arc;

use alloy_primitives::{Address, FixedBytes, U256};
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

/// Same address on both chains (PolyBridger-style).
const BRIDGE: &str = "0x1111111111111111111111111111111111111111";
const URL_ETH: &str = "mock://ethereum";
const URL_POLYGON: &str = "mock://polygon";
const BLOCK_ETH: u64 = 21_000_000;
const BLOCK_POLYGON: u64 = 50_000_000;
/// Cancun-era timestamp so SpecId stays PUSH0-safe for solc prague bytecode.
const TS_CANCUN: &str = "0x65f5e100";

alloy_sol_types::sol! {
    interface ForkHarness {
        function setup() external;
        function actionFork(string calldata url, uint256 blockNumber) external;
        function actionForkAndReadBridge(string calldata url, uint256 blockNumber) external;
        function actionForkStoreBridge(
            string calldata url,
            uint256 blockNumber,
            bytes32 value
        ) external;
        function actionForkDealBridge(
            string calldata url,
            uint256 blockNumber,
            uint256 value
        ) external;
        function actionReadBridge() external;
        function actionSetTracked(uint256 value) external;
        function actionForkAndBumpTracked(
            string calldata url,
            uint256 blockNumber,
            uint256 delta
        ) external;
        function actionRecordOutflow(
            string calldata url,
            uint256 blockNumber,
            uint256 amount
        ) external;
        function actionRecordInflow(
            string calldata url,
            uint256 blockNumber,
            uint256 amount
        ) external;
        function invariant_conservation() external view;
        function getBlockNumber() external view returns (uint256);
        function getChainId() external view returns (uint256);
        function getLastSlot0() external view returns (bytes32);
        function getLastBalance() external view returns (uint256);
        function getTrackedValue() external view returns (uint256);
        function getTotalOutflow() external view returns (uint256);
        function getTotalInflow() external view returns (uint256);
    }
}

fn mock_fork_setup(
    transport: &MockTransport,
    url: &str,
    block_number: u64,
    chain_id_hex: &str,
    timestamp_hex: &str,
) {
    let chain_id_payload = json!([
        {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
    ]);
    let block_payload = json!([
        {
            "jsonrpc":"2.0",
            "id":0,
            "method":"eth_getBlockByNumber",
            "params":[json!(format!("0x{block_number:x}")), json!(false)]
        }
    ]);
    transport.mock_response(
        url,
        &chain_id_payload,
        json!([{"jsonrpc":"2.0","id":0,"result":chain_id_hex}]),
    );
    transport.mock_response(
        url,
        &block_payload,
        json!([{
            "jsonrpc":"2.0",
            "id":0,
            "result":{
                "number": format!("0x{block_number:x}"),
                "timestamp": timestamp_hex,
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }
        }]),
    );
}

fn mock_bridge_account(
    transport: &MockTransport,
    url: &str,
    block: u64,
    balance_hex: &str,
    slot0_hex: &str,
) {
    let block_hex = format!("0x{block:x}");
    let basic_payload = json!([
        {
            "jsonrpc": "2.0",
            "id": 0,
            "method": "eth_getBalance",
            "params": [BRIDGE, block_hex]
        },
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getTransactionCount",
            "params": [BRIDGE, block_hex]
        },
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "eth_getCode",
            "params": [BRIDGE, block_hex]
        }
    ]);
    transport.mock_response(
        url,
        &basic_payload,
        json!([
            {"jsonrpc": "2.0", "id": 0, "result": balance_hex},
            {"jsonrpc": "2.0", "id": 1, "result": "0x0"},
            {"jsonrpc": "2.0", "id": 2, "result": "0x"}
        ]),
    );

    let storage_payload = json!([
        {
            "jsonrpc": "2.0",
            "id": 0,
            "method": "eth_getStorageAt",
            "params": [BRIDGE, "0x0", block_hex]
        }
    ]);
    transport.mock_response(
        url,
        &storage_payload,
        json!([{"jsonrpc": "2.0", "id": 0, "result": slot0_hex}]),
    );
}

fn setup_eth_polygon_forks(transport: &MockTransport) {
    // Ethereum: chain id 1, slot0 = 1, balance = 100
    mock_fork_setup(transport, URL_ETH, BLOCK_ETH, "0x1", TS_CANCUN);
    mock_bridge_account(transport, URL_ETH, BLOCK_ETH, "0x64", "0x1");

    // Polygon: chain id 137, slot0 = 2, balance = 200
    mock_fork_setup(transport, URL_POLYGON, BLOCK_POLYGON, "0x89", TS_CANCUN);
    mock_bridge_account(transport, URL_POLYGON, BLOCK_POLYGON, "0xc8", "0x2");
}

fn deploy_harness(transport: MockTransport) -> (Chain, Address) {
    let initcode = load_initcode("fixtures/evm/cheatcodes", "ForkHarness.sol:ForkHarness");

    let config = ChainConfig::default()
        .with_transport(Arc::new(transport))
        .with_fork_defaults(ForkDBConfig::new(""));
    let mut chain = Chain::new(config).unwrap();
    let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
    assert!(deployment.result.success, "deployment must succeed");
    let target = deployment.address.unwrap();
    let setup = chain.setup(SetupInput::new(target)).unwrap();
    assert!(setup.result.success, "setup must succeed");
    (chain, target)
}

fn u256_to_bytes32(value: U256) -> FixedBytes<32> {
    FixedBytes::from(value.to_be_bytes())
}

/// Full multi-fork call sequence in a single `exec`, the same shape as one
/// fuzzer corpus item: fork A, mutate A, fork B, read B, fork A again, read A.
#[test]
fn multi_fork_call_sequence_isolates_remote_state() {
    let transport = MockTransport::default();
    setup_eth_polygon_forks(&transport);
    let (mut chain, target) = deploy_harness(transport);

    let mutated_slot = u256_to_bytes32(U256::from(99));
    let mutated_balance = U256::from(999);

    // One call sequence (single exec):
    // 1. Fork Ethereum and read remote bridge (slot0=1, balance=100)
    // 2. Mutate Ethereum bridge storage to 99
    // 3. Mutate Ethereum bridge balance to 999
    // 4. Fork Polygon and read remote bridge (slot0=2, balance=200)
    // 5. Switch back to Ethereum (no re-read yet)
    // 6. Read bridge on active Ethereum fork (mutations must persist)
    let sequence = [
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkAndReadBridgeCall::new((
                URL_ETH.to_string(),
                U256::from(BLOCK_ETH),
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkStoreBridgeCall::new((
                URL_ETH.to_string(),
                U256::from(BLOCK_ETH),
                mutated_slot,
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkDealBridgeCall::new((
                URL_ETH.to_string(),
                U256::from(BLOCK_ETH),
                mutated_balance,
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkAndReadBridgeCall::new((
                URL_POLYGON.to_string(),
                U256::from(BLOCK_POLYGON),
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((URL_ETH.to_string(), U256::from(BLOCK_ETH)))
                .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionReadBridgeCall::new(()).abi_encode(),
        )),
    ];

    let execution = chain.exec(&sequence).unwrap();
    assert_eq!(execution.results.len(), 6);
    for (i, result) in execution.results.iter().enumerate() {
        assert!(
            result.success,
            "sequence step {i} must succeed: {:?}",
            result.output
        );
    }

    // After the sequence, last* fields hold the final Ethereum read.
    let txs = [
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getLastSlot0Call::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getLastBalanceCall::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getBlockNumberCall::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getChainIdCall::new(()).abi_encode(),
        )),
    ];
    let execution = chain.exec(&txs).unwrap();
    assert!(
        execution.results.iter().all(|r| r.success),
        "post-sequence reads must succeed"
    );

    let slot0 = ForkHarness::getLastSlot0Call::abi_decode_returns(
        &execution.results[0].output.clone().unwrap(),
    )
    .unwrap();
    let balance = ForkHarness::getLastBalanceCall::abi_decode_returns(
        &execution.results[1].output.clone().unwrap(),
    )
    .unwrap();
    let block_number = ForkHarness::getBlockNumberCall::abi_decode_returns(
        &execution.results[2].output.clone().unwrap(),
    )
    .unwrap();
    let chain_id = ForkHarness::getChainIdCall::abi_decode_returns(
        &execution.results[3].output.clone().unwrap(),
    )
    .unwrap();

    assert_eq!(
        slot0, mutated_slot,
        "ethereum store mutation must survive polygon switch inside the sequence"
    );
    assert_eq!(
        balance, mutated_balance,
        "ethereum deal mutation must survive polygon switch inside the sequence"
    );
    assert_eq!(
        block_number,
        U256::from(BLOCK_ETH),
        "active fork after sequence must be ethereum block"
    );
    assert_eq!(
        chain_id,
        U256::from(1),
        "active fork after sequence must be ethereum chain id"
    );
}

/// Call sequence that only switches env (no remote mutations): eth -> polygon
/// -> eth, checking chain id and block number after the full sequence.
#[test]
fn multi_fork_call_sequence_switches_env() {
    let transport = MockTransport::default();
    setup_eth_polygon_forks(&transport);
    let (mut chain, target) = deploy_harness(transport);

    // Single exec sequence: eth -> polygon -> eth
    let sequence = [
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((URL_ETH.to_string(), U256::from(BLOCK_ETH)))
                .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((URL_POLYGON.to_string(), U256::from(BLOCK_POLYGON)))
                .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((URL_ETH.to_string(), U256::from(BLOCK_ETH)))
                .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getBlockNumberCall::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getChainIdCall::new(()).abi_encode(),
        )),
    ];

    let execution = chain.exec(&sequence).unwrap();
    assert_eq!(execution.results.len(), 5);
    assert!(
        execution.results.iter().all(|r| r.success),
        "every step in the multi-fork sequence must succeed"
    );

    let block_number = ForkHarness::getBlockNumberCall::abi_decode_returns(
        &execution.results[3].output.clone().unwrap(),
    )
    .unwrap();
    let chain_id = ForkHarness::getChainIdCall::abi_decode_returns(
        &execution.results[4].output.clone().unwrap(),
    )
    .unwrap();
    assert_eq!(block_number, U256::from(BLOCK_ETH));
    assert_eq!(chain_id, U256::from(1));

    // Chain-level env must match the last fork in the sequence.
    assert_eq!(chain.cfg_env().chain_id, 1);
    assert_eq!(chain.block_env().number, U256::from(BLOCK_ETH));
    assert_eq!(chain.block_env().basefee, 0);
    assert_eq!(chain.block_env().gas_limit, u64::MAX);
}

/// Polygon read in the middle of a sequence must not see Ethereum mutations
/// written earlier in the same sequence. Capture polygon values via a second
/// short sequence after the multi-fork sequence would re-run state; instead
/// record polygon values into last* during the sequence itself.
#[test]
fn multi_fork_call_sequence_polygon_unaffected_by_prior_eth_mutations() {
    let transport = MockTransport::default();
    setup_eth_polygon_forks(&transport);
    let (mut chain, target) = deploy_harness(transport);

    let mutated_slot = u256_to_bytes32(U256::from(99));
    let mutated_balance = U256::from(999);

    // Sequence ends on Polygon after mutating Ethereum, so last* holds polygon
    // remote values (slot0=2, balance=200).
    let sequence = [
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkStoreBridgeCall::new((
                URL_ETH.to_string(),
                U256::from(BLOCK_ETH),
                mutated_slot,
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkDealBridgeCall::new((
                URL_ETH.to_string(),
                U256::from(BLOCK_ETH),
                mutated_balance,
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkAndReadBridgeCall::new((
                URL_POLYGON.to_string(),
                U256::from(BLOCK_POLYGON),
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getLastSlot0Call::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getLastBalanceCall::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getChainIdCall::new(()).abi_encode(),
        )),
    ];

    let execution = chain.exec(&sequence).unwrap();
    assert_eq!(execution.results.len(), 6);
    assert!(
        execution.results.iter().all(|r| r.success),
        "sequence must succeed end-to-end"
    );

    let slot0 = ForkHarness::getLastSlot0Call::abi_decode_returns(
        &execution.results[3].output.clone().unwrap(),
    )
    .unwrap();
    let balance = ForkHarness::getLastBalanceCall::abi_decode_returns(
        &execution.results[4].output.clone().unwrap(),
    )
    .unwrap();
    let chain_id = ForkHarness::getChainIdCall::abi_decode_returns(
        &execution.results[5].output.clone().unwrap(),
    )
    .unwrap();

    assert_eq!(
        slot0,
        u256_to_bytes32(U256::from(2)),
        "polygon slot0 must remain remote value 2 inside the sequence"
    );
    assert_eq!(
        balance,
        U256::from(200),
        "polygon balance must remain remote value 200 inside the sequence"
    );
    assert_eq!(chain_id, U256::from(0x89), "active chain must be polygon");
}

/// Local harness storage must survive fork switches so campaigns can track
/// ghost state (amounts, counters) while remote state stays isolated per chain.
#[test]
fn harness_local_state_survives_fork_switches_in_sequence() {
    let transport = MockTransport::default();
    setup_eth_polygon_forks(&transport);
    let (mut chain, target) = deploy_harness(transport);

    // One sequence:
    // 1. Seed trackedValue = 1000 on empty sandbox
    // 2. Fork eth and bump +7 (tracked = 1007)
    // 3. Fork polygon and bump +3 (tracked = 1010)
    // 4. Fork eth again (no bump)
    // 5. Read trackedValue (must still be 1010)
    let sequence = [
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionSetTrackedCall::new((U256::from(1000),)).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkAndBumpTrackedCall::new((
                URL_ETH.to_string(),
                U256::from(BLOCK_ETH),
                U256::from(7),
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkAndBumpTrackedCall::new((
                URL_POLYGON.to_string(),
                U256::from(BLOCK_POLYGON),
                U256::from(3),
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((URL_ETH.to_string(), U256::from(BLOCK_ETH)))
                .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getTrackedValueCall::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getChainIdCall::new(()).abi_encode(),
        )),
    ];

    let execution = chain.exec(&sequence).unwrap();
    assert_eq!(execution.results.len(), 6);
    assert!(
        execution.results.iter().all(|r| r.success),
        "harness state sequence must succeed: {:?}",
        execution
            .results
            .iter()
            .map(|r| (r.success, r.output.clone()))
            .collect::<Vec<_>>()
    );

    let tracked = ForkHarness::getTrackedValueCall::abi_decode_returns(
        &execution.results[4].output.clone().unwrap(),
    )
    .unwrap();
    let chain_id = ForkHarness::getChainIdCall::abi_decode_returns(
        &execution.results[5].output.clone().unwrap(),
    )
    .unwrap();

    assert_eq!(
        tracked,
        U256::from(1010),
        "trackedValue must accumulate across eth and polygon forks"
    );
    assert_eq!(chain_id, U256::from(1), "sequence ends on ethereum");
}

/// Value conservation across chains: record outflow on eth, inflow on polygon,
/// and assert local totals stay consistent after switches (ghost accounting).
#[test]
fn harness_value_conservation_across_chains_in_sequence() {
    let transport = MockTransport::default();
    setup_eth_polygon_forks(&transport);
    let (mut chain, target) = deploy_harness(transport);

    let amount_a = U256::from(100);
    let amount_b = U256::from(40);

    // Sequence models a bridge campaign:
    // 1. Lock 100 on eth (outflow)
    // 2. Mint 100 on polygon (inflow) -> conserved
    // 3. Lock 40 more on eth
    // 4. Mint 40 on polygon -> conserved again
    // 5. Switch to eth and read totals
    // 6. Run conservation invariant
    let sequence = [
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionRecordOutflowCall::new((
                URL_ETH.to_string(),
                U256::from(BLOCK_ETH),
                amount_a,
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionRecordInflowCall::new((
                URL_POLYGON.to_string(),
                U256::from(BLOCK_POLYGON),
                amount_a,
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionRecordOutflowCall::new((
                URL_ETH.to_string(),
                U256::from(BLOCK_ETH),
                amount_b,
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionRecordInflowCall::new((
                URL_POLYGON.to_string(),
                U256::from(BLOCK_POLYGON),
                amount_b,
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((URL_ETH.to_string(), U256::from(BLOCK_ETH)))
                .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getTotalOutflowCall::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getTotalInflowCall::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getTrackedValueCall::new(()).abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::invariant_conservationCall::new(()).abi_encode(),
        )),
    ];

    let execution = chain.exec(&sequence).unwrap();
    assert_eq!(execution.results.len(), 9);
    assert!(
        execution.results.iter().all(|r| r.success),
        "conservation sequence must succeed: {:?}",
        execution
            .results
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.success, r.output.clone()))
            .collect::<Vec<_>>()
    );

    let total_out = ForkHarness::getTotalOutflowCall::abi_decode_returns(
        &execution.results[5].output.clone().unwrap(),
    )
    .unwrap();
    let total_in = ForkHarness::getTotalInflowCall::abi_decode_returns(
        &execution.results[6].output.clone().unwrap(),
    )
    .unwrap();
    let tracked = ForkHarness::getTrackedValueCall::abi_decode_returns(
        &execution.results[7].output.clone().unwrap(),
    )
    .unwrap();

    let expected = amount_a + amount_b;
    assert_eq!(total_out, expected, "outflow must sum across eth hops");
    assert_eq!(total_in, expected, "inflow must sum across polygon hops");
    assert_eq!(
        tracked, expected,
        "trackedValue ghost must equal total bridged amount"
    );
    assert_eq!(total_out, total_in, "value must be conserved across chains");
}

/// Unbalanced conservation must fail the invariant after a multi-fork sequence
/// (outflow without matching inflow).
#[test]
fn harness_value_conservation_invariant_fails_when_unbalanced() {
    let transport = MockTransport::default();
    setup_eth_polygon_forks(&transport);
    let (mut chain, target) = deploy_harness(transport);

    // Outflow on eth only; never record matching inflow on polygon.
    let sequence = [
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionRecordOutflowCall::new((
                URL_ETH.to_string(),
                U256::from(BLOCK_ETH),
                U256::from(50),
            ))
            .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((URL_POLYGON.to_string(), U256::from(BLOCK_POLYGON)))
                .abi_encode(),
        )),
        Transaction::new(target).calldata(Bytes::from(
            ForkHarness::invariant_conservationCall::new(()).abi_encode(),
        )),
    ];

    let execution = chain.exec(&sequence).unwrap();
    assert!(execution.results[0].success, "outflow action must succeed");
    assert!(execution.results[1].success, "polygon fork must succeed");
    assert!(
        !execution.results[2].success,
        "invariant_conservation must fail when inflow != outflow"
    );
    assert!(
        execution.broken_invariants[2].is_empty(),
        "an `assert` panic is not a broken invariant"
    );
}
