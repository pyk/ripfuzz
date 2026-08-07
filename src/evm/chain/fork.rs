//! Forked chain initialisation helpers (tests and library convenience).

use std::sync::Arc;

use alloy_primitives::U256;
use anyhow::{Context as _, Result};
use revm::context_interface::block::BlobExcessGasAndPrice;

use crate::evm::chain::{Chain, ChainConfig};
use crate::evm::forkdb::{ForkDBConfig, Transport};

impl Chain {
    /// Create a chain that is already forked (used in tests).
    ///
    /// Prefer `vm.fork` from harness code for campaigns. This helper builds an
    /// empty chain and immediately selects the given fork via the multi-fork
    /// database layer.
    pub fn fork_with_transport(
        chain_config: ChainConfig,
        forkdb_config: ForkDBConfig,
        transport: impl Transport + 'static,
    ) -> Result<Self> {
        let transport: Arc<dyn Transport> = Arc::new(transport);
        let chain_config = chain_config
            .with_fork_defaults(forkdb_config.clone())
            .with_transport(transport.clone());
        let mut chain = Self::empty(chain_config);

        let url = forkdb_config.url.clone();
        let block_number = forkdb_config.block_number;
        let db = chain.database.as_mut().context("database unavailable")?;
        let env = db
            .fork(
                &url,
                block_number,
                forkdb_config,
                Some(transport),
                chain.local_registry.clone(),
            )
            .context("creating fork")?;

        chain.cfg_env.chain_id = env.chain_id;
        chain.cfg_env.set_spec_and_mainnet_gas_params(env.spec_id);
        chain.block_env.number = U256::from(env.block_number);
        chain.block_env.timestamp = env.timestamp;
        chain.block_env.beneficiary = env.beneficiary;
        chain.block_env.difficulty = env.difficulty;
        chain.block_env.prevrandao = env.prevrandao;
        chain.block_env.gas_limit = u64::MAX;
        chain.block_env.basefee = 0;
        let excess = env.excess_blob_gas.unwrap_or(0);
        chain.block_env.blob_excess_gas_and_price =
            Some(BlobExcessGasAndPrice::new_with_spec(excess, env.spec_id));

        chain.cheatcode_state.block.chain_id = Some(U256::from(env.chain_id));
        chain.cheatcode_state.block.number = Some(U256::from(env.block_number));
        chain.cheatcode_state.block.timestamp = Some(env.timestamp);
        chain.cheatcode_state.block.beneficiary = Some(env.beneficiary);
        chain.cheatcode_state.block.prevrandao = env.prevrandao;

        Ok(chain)
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256, U256};
    use revm::DatabaseRef;
    use revm::primitives::Bytes;
    use revm::primitives::hardfork::SpecId;
    use serde_json::json;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput, SetupInput};
    use crate::evm::cheatcode::VM_ADDRESS;
    use crate::evm::forkdb::{ForkDBConfig, MockTransport};
    use crate::foundry::{ArtifactId, Project};

    use super::*;

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

    /// Chain::fork must seed the default deployer with U256::MAX balance,
    /// just like Chain::new, so that setup and deployment transactions
    /// never fail due to insufficient funds.
    #[test]
    fn chain_fork_seeds_deployer_with_max_balance() {
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

        let config = ForkDBConfig::new(url).block_number(1);
        let chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();
        assert_eq!(chain.deployer(), DEFAULT_DEPLOYER);

        let chain_id_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        let block_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]}
        ]);
        assert_eq!(
            transport.call_count(url, &chain_id_payload),
            1,
            "Chain::fork must fetch chain_id once"
        );
        assert_eq!(
            transport.call_count(url, &block_payload),
            1,
            "Chain::fork must fetch block once"
        );

        let db = chain.database().unwrap();
        let info = db.basic_ref(DEFAULT_DEPLOYER).unwrap().unwrap();
        assert_eq!(
            info.balance,
            U256::MAX,
            "deployer must be seeded with U256::MAX in Chain::fork"
        );
    }

    /// Chain::fork must inject a dummy contract at the ripfuzz VM address so
    /// that Solidity `extcodesize` checks do not revert when a harness contract
    /// calls cheatcodes during deployment or setup.
    #[test]
    fn chain_fork_injects_vm_address() {
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

        let config = ForkDBConfig::new(url).block_number(1);
        let chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();

        let chain_id_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        let block_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]}
        ]);
        assert_eq!(
            transport.call_count(url, &chain_id_payload),
            1,
            "Chain::fork must fetch chain_id once"
        );
        assert_eq!(
            transport.call_count(url, &block_payload),
            1,
            "Chain::fork must fetch block once"
        );

        let db = chain.database().unwrap();
        let info = db.basic_ref(VM_ADDRESS).unwrap().unwrap();
        let code = info.code.as_ref().unwrap();
        assert!(
            !code.is_empty(),
            "Chain::fork must inject non-empty code at VM_ADDRESS so extcodesize checks pass"
        );
    }

    /// Regression: Chain::fork must derive the EVM spec from the forked
    /// block number and timestamp instead of hardcoding AMSTERDAM. A
    /// mainnet block past Cancun activation must use SpecId::CANCUN.
    #[test]
    fn chain_fork_uses_correct_spec_for_mainnet_cancun() {
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

        let config = ForkDBConfig::new(url).block_number(20_000_000);
        let chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();
        assert_eq!(
            chain.cfg_env().spec,
            SpecId::CANCUN,
            "fork at mainnet block 20_000_000 must use Cancun spec"
        );
    }

    /// Chain::fork must resolve the correct spec for an OP-stack chain.
    /// Base mainnet (chain_id 8453) at the Ecotone timestamp must use
    /// the bundled Cancun spec.
    #[test]
    fn chain_fork_uses_correct_spec_for_base_mainnet() {
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

        let config = ForkDBConfig::new(url).block_number(9_000_000);
        let chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();
        assert_eq!(
            chain.cfg_env().spec,
            SpecId::CANCUN,
            "fork at Base mainnet post-Ecotone must use Cancun spec"
        );
    }

    /// Regression: Chain::fork must zero `basefee` so that transactions with
    /// `gas_price = 0` (the default used by deploy/call) do not fail revm
    /// validation with `GasPriceLessThanBasefee`.
    #[test]
    fn chain_fork_zeros_basefee() {
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

        let config = ForkDBConfig::new(url).block_number(1);
        let mut chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();

        assert_eq!(
            chain.block_env().basefee,
            0,
            "Chain::fork must zero basefee so gas_price=0 transactions validate"
        );

        chain.seed_account(Address::ZERO, U256::ZERO).unwrap();
        let tx_result = chain
            .call(DEFAULT_DEPLOYER, VM_ADDRESS, U256::ZERO, Bytes::new())
            .unwrap();
        assert!(
            tx_result.success,
            "call with gas_price=0 must succeed in fork mode"
        );
    }

    /// Regression: Chain::fork must cache the fork block hash so that the
    /// BLOCKHASH opcode for the fork block number resolves locally instead of
    /// triggering an eth_getBlockByNumber RPC call.
    #[test]
    fn chain_fork_caches_fork_block_hash() {
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

        let config = ForkDBConfig::new(url).block_number(1);
        let chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();
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

    /// Regression: Chain::fork must derive the blob base fee update fraction from
    /// the resolved `SpecId` instead of hardcoding the Cancun mainnet value
    /// (`3338477`). Forking a Prague block on mainnet should use the Prague
    /// fraction (`5007716`), which produces a different blob gasprice for the same
    /// excess blob gas.
    #[test]
    fn chain_fork_uses_spec_aware_blob_fraction() {
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

        let config = ForkDBConfig::new(url).block_number(1);
        let chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();
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

    /// Regression: Chain::fork must disable the initcode size limit so that
    /// large factory contracts or inlined deployment logic that runs during
    /// forked setup does not revert with MaxInitCodeSizeExceeded.
    #[test]
    fn chain_fork_allows_unlimited_initcode_size() {
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

        let config = ForkDBConfig::new(url).block_number(1);
        let chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();
        assert_eq!(
            chain.cfg_env().limit_contract_initcode_size,
            Some(usize::MAX),
            "Chain::fork must disable initcode size limit just like Chain::new"
        );
    }

    /// Regression: Chain::fork must validate that the block returned by
    /// eth_getBlockByNumber matches the requested block number. A lagging node,
    /// reorg race, or misconfigured proxy could return a different block.
    #[test]
    fn chain_fork_rejects_mismatched_block_number() {
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

        let config = ForkDBConfig::new(url).block_number(100);
        let result = Chain::fork_with_transport(ChainConfig::default(), config, transport.clone());
        assert!(
            result.is_err(),
            "Chain::fork must reject a block whose number does not match the requested height"
        );
        let err_msg = format!("{:#}", result.unwrap_err());
        assert!(
            err_msg.contains("100"),
            "error message must mention the requested block number: {err_msg}"
        );
        assert!(
            err_msg.contains("1"),
            "error message must mention the returned block number: {err_msg}"
        );
    }

    /// Regression: Chain::fork must set the block gas limit to `u64::MAX`
    /// so that deployment transactions with gas limit `u64::MAX` do not fail
    /// revm validation with `CallerGasLimitMoreThanBlock`.
    #[test]
    fn chain_fork_allows_deployment_with_max_gas_limit() {
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

        // Mock the ForkDB response for the harness contract address.
        let target_addr_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xb48bd837cb11a87bead45ea4b7ea3164e8af71f2","0x1438f2d"]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0xb48bd837cb11a87bead45ea4b7ea3164e8af71f2","0x1438f2d"]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0xb48bd837cb11a87bead45ea4b7ea3164e8af71f2","0x1438f2d"]}
        ]);
        transport.mock_response(
            url,
            &target_addr_payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0"},
                {"jsonrpc":"2.0","id":1,"result":"0x0"},
                {"jsonrpc":"2.0","id":2,"result":"0x"}
            ]),
        );

        // Mock the ForkDB response for Address::ZERO (coinbase).
        let zero_addr_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0x0000000000000000000000000000000000000000","0x1438f2d"]},
            {"jsonrpc":"2.0","id":1,"method":"eth_getTransactionCount","params":["0x0000000000000000000000000000000000000000","0x1438f2d"]},
            {"jsonrpc":"2.0","id":2,"method":"eth_getCode","params":["0x0000000000000000000000000000000000000000","0x1438f2d"]}
        ]);
        transport.mock_response(
            url,
            &zero_addr_payload,
            json!([
                {"jsonrpc":"2.0","id":0,"result":"0x0"},
                {"jsonrpc":"2.0","id":1,"result":"0x0"},
                {"jsonrpc":"2.0","id":2,"result":"0x"}
            ]),
        );

        let config = ForkDBConfig::new(url).block_number(21_204_781);
        let mut chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();

        // Load a simple contract from the fixture project.
        let project = Project::new("fixtures/harness-contract-deployment");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id =
            ArtifactId::try_from("test/EmptyChainNoSetup.sol:EmptyChainNoSetup").unwrap();
        let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();

        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(
            deployment.result.success,
            "deployment must succeed on forked chain even when the real block has a limited gas limit"
        );
    }

    /// Regression: Chain::fork must set the chain_id on all transactions
    /// (deploy, call, setup) so that revm's `tx_chain_id_check` (enabled by
    /// default) does not reject the transaction with "invalid chain ID". This is
    /// especially important for non-mainnet chains like Base (chain_id 8453).
    #[test]
    fn chain_fork_deployment_sets_chain_id_on_tx() {
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

        let config = ForkDBConfig::new(url).block_number(9_000_000);
        let mut chain =
            Chain::fork_with_transport(ChainConfig::default(), config, transport.clone()).unwrap();

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
            "deployment must succeed on forked chain with non-mainnet chain_id (8453)"
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
        let target = setup_deployment.address.unwrap();
        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(
            setup.result.success,
            "setup must succeed on forked chain with non-mainnet chain_id (8453)"
        );
    }
}
