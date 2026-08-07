//! Fork mode regression tests via `rvm.fork`.
//!
//! Campaigns always start as an empty sandbox. These tests pin remote state
//! through the harness cheatcode rather than a library-side fork helper.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alloy_primitives::{Address, B256, U256};
    use alloy_sol_types::SolCall;
    use revm::DatabaseRef;
    use revm::primitives::Bytes;
    use revm::primitives::hardfork::SpecId;
    use serde_json::json;

    use crate::evm::ChainConfig;
    use crate::evm::Contract;
    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput, SetupInput, Transaction};
    use crate::evm::cheatcode::VM_ADDRESS;
    use crate::evm::forkdb::{ForkDBConfig, MockTransport};
    use crate::foundry::{ArtifactId, Project};

    alloy_sol_types::sol! {
        interface ForkHarness {
            function setup() external;
            function actionFork(string calldata url, uint256 blockNumber) external;
            function getBlockNumber() external view returns (uint256);
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
            {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[json!(format!("0x{block_number:x}")), json!(false)]}
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

    fn load_fork_harness() -> Contract {
        let project = Project::new("fixtures/harness-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = ArtifactId::try_from("src/ForkHarness.sol:ForkHarness").unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    /// Deploy ForkHarness on an empty chain with the given transport.
    fn deploy_harness(transport: MockTransport) -> (Chain, Address) {
        let contract = load_fork_harness();
        let config = ChainConfig::default()
            .with_transport(Arc::new(transport))
            .with_fork_defaults(ForkDBConfig::new(""));
        let mut chain = Chain::new(config).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();
        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");
        (chain, target)
    }

    /// Call `actionFork` and assert success.
    fn action_fork(chain: &mut Chain, target: Address, url: &str, block: u64) {
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((url.to_string(), U256::from(block))).abi_encode(),
        ))];
        let execution = chain.exec(&txs).unwrap();
        assert!(
            execution.results[0].success,
            "actionFork must succeed: {:?}",
            execution.results[0].output
        );
    }

    /// `rvm.fork` must keep the default deployer at U256::MAX balance so
    /// setup and deployment transactions never fail due to insufficient funds.
    #[test]
    fn rvm_fork_keeps_deployer_max_balance() {
        let transport = MockTransport::default();
        let url = "mock://test";

        mock_fork_setup(
            &transport,
            url,
            1,
            "0x1",
            json!({
                "number":"0x1",
                "timestamp":"0x1",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }),
        );

        let (mut chain, target) = deploy_harness(transport.clone());
        assert_eq!(chain.deployer(), DEFAULT_DEPLOYER);
        action_fork(&mut chain, target, url, 1);

        let chain_id_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        let block_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]}
        ]);
        assert_eq!(
            transport.call_count(url, &chain_id_payload),
            1,
            "rvm.fork must fetch chain_id once"
        );
        assert_eq!(
            transport.call_count(url, &block_payload),
            1,
            "rvm.fork must fetch block once"
        );

        let db = chain.database().unwrap();
        let info = db.basic_ref(DEFAULT_DEPLOYER).unwrap().unwrap();
        assert_eq!(
            info.balance,
            U256::MAX,
            "deployer must remain U256::MAX after rvm.fork"
        );
    }

    /// `rvm.fork` must keep non-empty code at the ripfuzz VM address so
    /// Solidity `extcodesize` checks pass for subsequent cheatcodes.
    #[test]
    fn rvm_fork_keeps_vm_address_code() {
        let transport = MockTransport::default();
        let url = "mock://test";

        mock_fork_setup(
            &transport,
            url,
            1,
            "0x1",
            json!({
                "number":"0x1",
                "timestamp":"0x1",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }),
        );

        let (mut chain, target) = deploy_harness(transport.clone());
        action_fork(&mut chain, target, url, 1);

        let db = chain.database().unwrap();
        let info = db.basic_ref(VM_ADDRESS).unwrap().unwrap();
        let code = info.code.as_ref().unwrap();
        assert!(
            !code.is_empty(),
            "rvm.fork must keep non-empty code at VM_ADDRESS"
        );
    }

    /// Regression: `rvm.fork` must derive the EVM spec from the forked
    /// block number and timestamp. A mainnet block past Cancun activation
    /// must use SpecId::CANCUN.
    #[test]
    fn rvm_fork_uses_correct_spec_for_mainnet_cancun() {
        let transport = MockTransport::default();
        let url = "mock://test";

        mock_fork_setup(
            &transport,
            url,
            20_000_000,
            "0x1",
            json!({
                "number":"0x1312d00",
                "timestamp":"0x65f5e100",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }),
        );

        let (mut chain, target) = deploy_harness(transport);
        action_fork(&mut chain, target, url, 20_000_000);
        assert_eq!(
            chain.cfg_env().spec,
            SpecId::CANCUN,
            "fork at mainnet block 20_000_000 must use Cancun spec"
        );
    }

    /// `rvm.fork` must resolve the correct spec for an OP-stack chain.
    /// Base mainnet (chain_id 8453) at the Ecotone timestamp must use
    /// the bundled Cancun spec.
    #[test]
    fn rvm_fork_uses_correct_spec_for_base_mainnet() {
        let transport = MockTransport::default();
        let url = "mock://test";

        mock_fork_setup(
            &transport,
            url,
            9_000_000,
            "0x2105",
            json!({
                "number":"0x895440",
                "timestamp":"0x665fd100",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }),
        );

        let (mut chain, target) = deploy_harness(transport);
        action_fork(&mut chain, target, url, 9_000_000);
        assert_eq!(
            chain.cfg_env().spec,
            SpecId::CANCUN,
            "fork at Base mainnet post-Ecotone must use Cancun spec"
        );
    }

    /// Regression: `rvm.fork` must zero `basefee` so that transactions with
    /// `gas_price = 0` (the default used by deploy/call) do not fail revm
    /// validation with `GasPriceLessThanBasefee`.
    #[test]
    fn rvm_fork_zeros_basefee() {
        let transport = MockTransport::default();
        let url = "mock://test";

        mock_fork_setup(
            &transport,
            url,
            1,
            "0x1",
            json!({
                "number":"0x1",
                "timestamp":"0x1",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x1",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }),
        );

        let (mut chain, target) = deploy_harness(transport);
        action_fork(&mut chain, target, url, 1);

        assert_eq!(
            chain.block_env().basefee,
            0,
            "rvm.fork must zero basefee so gas_price=0 transactions validate"
        );

        chain.seed_account(Address::ZERO, U256::ZERO).unwrap();
        let tx_result = chain
            .call(DEFAULT_DEPLOYER, VM_ADDRESS, U256::ZERO, Bytes::new())
            .unwrap();
        assert!(
            tx_result.success,
            "call with gas_price=0 must succeed after rvm.fork"
        );
    }

    /// Regression: `rvm.fork` must cache the fork block hash so that the
    /// BLOCKHASH opcode for the fork block number resolves locally instead of
    /// triggering an eth_getBlockByNumber RPC call.
    #[test]
    fn rvm_fork_caches_fork_block_hash() {
        let transport = MockTransport::default();
        let url = "mock://test";
        let fork_hash = B256::from([0xab; 32]);

        mock_fork_setup(
            &transport,
            url,
            1,
            "0x1",
            json!({
                "number":"0x1",
                "timestamp":"0x1",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash": fork_hash
            }),
        );

        let (mut chain, target) = deploy_harness(transport.clone());
        action_fork(&mut chain, target, url, 1);
        let db = chain.database().unwrap();

        let hash = db.block_hash_ref(1).unwrap();
        assert_eq!(hash, fork_hash, "fork block hash must be cached");

        let extra_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]}
        ]);
        assert_eq!(
            transport.call_count(url, &extra_payload),
            1,
            "block_hash_ref must not trigger an extra RPC for the fork block"
        );
    }

    /// Regression: `rvm.fork` must derive the blob base fee update fraction from
    /// the resolved `SpecId` instead of hardcoding the Cancun mainnet value.
    #[test]
    fn rvm_fork_uses_spec_aware_blob_fraction() {
        let transport = MockTransport::default();
        let url = "mock://test";

        mock_fork_setup(
            &transport,
            url,
            1,
            "0x1",
            json!({
                "number":"0x1",
                "timestamp":"0x681b3057",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "excessBlobGas":"0x4c4b40"
            }),
        );

        let (mut chain, target) = deploy_harness(transport);
        action_fork(&mut chain, target, url, 1);
        let blob_info = chain.block_env().blob_excess_gas_and_price.unwrap();

        assert_eq!(
            blob_info.blob_gasprice, 2,
            "fork at Prague must use Prague blob base fee update fraction"
        );
        assert_eq!(
            blob_info.excess_blob_gas, 5_000_000,
            "excess_blob_gas must match RPC response"
        );
    }

    /// Regression: `rvm.fork` must preserve the unlimited initcode size limit
    /// from the empty sandbox so large factory contracts do not revert with
    /// MaxInitCodeSizeExceeded.
    #[test]
    fn rvm_fork_preserves_unlimited_initcode_size() {
        let transport = MockTransport::default();
        let url = "mock://test";

        mock_fork_setup(
            &transport,
            url,
            1,
            "0x1",
            json!({
                "number":"0x1",
                "timestamp":"0x1",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }),
        );

        let (mut chain, target) = deploy_harness(transport);
        action_fork(&mut chain, target, url, 1);
        assert_eq!(
            chain.cfg_env().limit_contract_initcode_size,
            Some(usize::MAX),
            "rvm.fork must preserve unlimited initcode size"
        );
    }

    /// Regression: `rvm.fork` must validate that the block returned by
    /// eth_getBlockByNumber matches the requested block number.
    #[test]
    fn rvm_fork_rejects_mismatched_block_number() {
        let transport = MockTransport::default();
        let url = "mock://test";

        let chain_id_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        let block_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[json!("0x64"), json!(false)]}
        ]);
        transport.mock_response(
            url,
            &chain_id_payload,
            json!([{"jsonrpc":"2.0","id":0,"result":"0x1"}]),
        );
        // Return block number 1 when block 100 was requested.
        transport.mock_response(
            url,
            &block_payload,
            json!([{"jsonrpc":"2.0","id":0,"result":{
                "number":"0x1",
                "timestamp":"0x1",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }}]),
        );

        let (mut chain, target) = deploy_harness(transport);
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((url.to_string(), U256::from(100))).abi_encode(),
        ))];
        let execution = chain.exec(&txs).unwrap();
        assert!(
            !execution.results[0].success,
            "rvm.fork must reject a block whose number does not match the requested height"
        );
        let output = execution.results[0]
            .output
            .as_ref()
            .expect("fork mismatch must return revert data");
        let msg = String::from_utf8_lossy(output);
        assert!(
            msg.contains("100") && msg.contains("1"),
            "error must mention requested and returned block numbers: {msg}"
        );
    }

    /// Regression: `rvm.fork` must set the block gas limit to `u64::MAX`
    /// so that deployment transactions with gas limit `u64::MAX` do not fail
    /// revm validation with `CallerGasLimitMoreThanBlock`.
    #[test]
    fn rvm_fork_allows_deployment_with_max_gas_limit() {
        let transport = MockTransport::default();
        let url = "mock://test";

        // Use a realistic mainnet block gas limit (30M) to reproduce the bug.
        mock_fork_setup(
            &transport,
            url,
            21_204_781,
            "0x1",
            json!({
                "number":"0x1438f2d",
                "timestamp":"0x6739e2b3",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0x1c9c380",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "excessBlobGas":"0x0",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }),
        );

        let (mut chain, target) = deploy_harness(transport.clone());
        action_fork(&mut chain, target, url, 21_204_781);

        assert_eq!(
            chain.block_env().gas_limit,
            u64::MAX,
            "rvm.fork must set block gas limit to u64::MAX"
        );

        // Load a simple contract from the fixture project and deploy after fork.
        let project = Project::new("fixtures/harness-contract-deployment");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id =
            ArtifactId::try_from("test/EmptyChainNoSetup.sol:EmptyChainNoSetup").unwrap();
        let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(
            deployment.result.success,
            "deployment must succeed after rvm.fork even when the real block has a limited gas limit"
        );
    }

    /// Regression: after `rvm.fork`, subsequent transactions must use the
    /// forked chain_id so revm's `tx_chain_id_check` does not reject them.
    /// Especially important for non-mainnet chains like Base (chain_id 8453).
    #[test]
    fn rvm_fork_deployment_sets_chain_id_on_tx() {
        let transport = MockTransport::default();
        let url = "mock://test";

        // Use Base mainnet chain_id (8453 = 0x2105) to reproduce the original bug.
        mock_fork_setup(
            &transport,
            url,
            9_000_000,
            "0x2105",
            json!({
                "number":"0x895440",
                "timestamp":"0x665fd100",
                "miner":"0x0000000000000000000000000000000000000000",
                "gasLimit":"0xffffffffffffffff",
                "baseFeePerGas":"0x0",
                "difficulty":"0x0",
                "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                "excessBlobGas":"0x0",
                "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
            }),
        );

        let (mut chain, target) = deploy_harness(transport);
        action_fork(&mut chain, target, url, 9_000_000);

        // Verify the chain is recognized as Base (non-mainnet chain_id).
        assert_eq!(
            chain.cfg_env().chain_id,
            8453,
            "fork must use the real chain_id (8453)"
        );

        // --- Deploy: verify deployment sets chain_id on the TxEnv ---
        let project = Project::new("fixtures/harness-contract-deployment");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id =
            ArtifactId::try_from("test/EmptyChainNoSetup.sol:EmptyChainNoSetup").unwrap();
        let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(
            deployment.result.success,
            "deployment must succeed after rvm.fork with non-mainnet chain_id (8453)"
        );

        // --- Setup: verify setup sets chain_id on the TxEnv ---
        let setup_artifact_id =
            ArtifactId::try_from("test/EmptyChainCheatcodeInSetup.sol:EmptyChainCheatcodeInSetup")
                .unwrap();
        let setup_contract = Contract::try_get(&artifacts, &setup_artifact_id).unwrap();
        let setup_deployment = chain
            .deploy(DeployInput::new(&setup_contract.initcode))
            .unwrap();
        assert!(
            setup_deployment.result.success,
            "deployment of setup fixture must succeed"
        );
        let setup_target = setup_deployment.address.unwrap();
        let setup = chain.setup(SetupInput::new(setup_target)).unwrap();
        assert!(
            setup.result.success,
            "setup must succeed after rvm.fork with non-mainnet chain_id (8453)"
        );
    }
}
