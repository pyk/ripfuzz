//! Forked chain initialisation.

use alloy_primitives::U256;
use anyhow::{Context as _, Result, ensure};
use revm::{
    bytecode::Bytecode,
    context::{BlockEnv, CfgEnv},
    context_interface::block::BlobExcessGasAndPrice,
    database::CacheDB,
    primitives::Bytes,
    state::AccountInfo,
};

use crate::evm::chain::{Chain, DEFAULT_DEPLOYER};
use crate::evm::cheatcode::VM_ADDRESS;
use crate::evm::database::{Database, ForkDB};
use crate::evm::forkdb::{Config, Request, Response, Transport};

impl Chain {
    /// Create a new forked EVM pinned to a remote block.
    pub fn fork(config: Config, block_number: u64) -> Result<Self> {
        let agent_cfg = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_millis(config.timeout_ms)))
            .build();
        let agent = ureq::Agent::new_with_config(agent_cfg);
        Self::fork_with_transport(config, agent, block_number)
    }

    /// Create a forked EVM with a custom transport (used in tests).
    pub fn fork_with_transport(
        config: Config,
        transport: impl Transport + 'static,
        block_number: u64,
    ) -> Result<Self> {
        let url_hash = crate::evm::forkdb::url_hash(&config.url);
        let backend = crate::evm::forkdb::SharedBackend::new_with_transport(config, transport);

        // resolve chain_id so every subsequent cache key is scoped.
        let mut responses = backend
            .fetch_or_wait(&[Request::GetChainId { url_hash }])
            .with_context(|| "fetching chain id for fork")?;
        let chain_id = responses
            .pop()
            .and_then(|r| match r {
                Response::ChainId(v) => Some(v),
                _ => None,
            })
            .context("missing ChainId response")?;

        // fetch the fork block using the real chain_id.
        let mut responses = backend
            .fetch_or_wait(&[Request::GetBlockByNumber {
                chain_id,
                block: block_number,
                full_tx: false,
            }])
            .with_context(|| format!("fetching fork block {block_number}"))?;
        let block = responses
            .pop()
            .and_then(|r| match r {
                Response::BlockByNumber(b) => Some(b),
                _ => None,
            })
            .context("missing BlockByNumber response")?;

        let returned_number = block.number.to::<u64>();
        ensure!(
            returned_number == block_number,
            "RPC returned block {returned_number} but requested block {block_number}"
        );

        let spec_id = crate::evm::specs::get_spec_id(chain_id, block.timestamp.to());

        let mut cfg_env = CfgEnv::default();
        cfg_env.chain_id = chain_id;
        cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        cfg_env.disable_nonce_check = true;
        cfg_env.disable_eip3607 = true;
        cfg_env.disable_base_fee = true;
        cfg_env.limit_contract_code_size = Some(usize::MAX);
        cfg_env.limit_contract_initcode_size = Some(usize::MAX);
        cfg_env.set_spec_and_mainnet_gas_params(spec_id);

        let fork_db = ForkDB::new(backend.clone(), block_number, chain_id);
        let mut database = CacheDB::new(fork_db);

        // Pre-cache the fork block hash so the BLOCKHASH opcode does not
        // trigger an unnecessary RPC call.
        if let Some(hash) = block.hash {
            database
                .cache
                .block_hashes
                .insert(U256::from(block.number), hash);
        }

        let mut block_env = BlockEnv {
            number: U256::from(block.number),
            beneficiary: block.coinbase,
            timestamp: U256::from(block.timestamp),
            gas_limit: block.gas_limit.to(),
            // Zero basefee so that transactions with gas_price = 0 (the default
            // for deploy/call) do not fail validation with GasPriceLessThanBasefee.
            basefee: 0,
            difficulty: block.difficulty,
            prevrandao: block.prevrandao,
            blob_excess_gas_and_price: None,
            slot_num: 0,
        };
        if let Some(excess) = block.excess_blob_gas {
            block_env.blob_excess_gas_and_price =
                Some(BlobExcessGasAndPrice::new_with_spec(excess.to(), spec_id));
        }

        // Set deployer balance
        let info = AccountInfo {
            balance: U256::MAX,
            nonce: 0,
            code_hash: revm::primitives::KECCAK_EMPTY,
            code: None,
            account_id: None,
        };
        database.insert_account_info(DEFAULT_DEPLOYER, info);

        // Insert a dummy VM contract so Solidity's `extcodesize` check passes
        // when a target calls raptor cheatcodes during deployment or setup.
        let vm_code = Bytecode::new_raw(Bytes::from_static(&[0x00]));
        database.insert_account_info(
            VM_ADDRESS,
            AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: vm_code.hash_slow(),
                code: Some(vm_code),
                account_id: None,
            },
        );

        Ok(Self {
            database: Some(Database::Fork(database)),
            block_env,
            cfg_env,
            deployer: DEFAULT_DEPLOYER,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256, U256};
    use revm::DatabaseRef;
    use revm::primitives::Bytes;
    use revm::primitives::hardfork::SpecId;
    use serde_json::json;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER};
    use crate::evm::cheatcode::VM_ADDRESS;
    use crate::evm::forkdb::{Config, MockTransport};

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

        let config = Config::new(url);
        let chain = Chain::fork_with_transport(config, transport.clone(), 1).unwrap();
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

    /// Chain::fork must inject a dummy contract at the raptor VM address so
    /// that Solidity `extcodesize` checks do not revert when a target contract
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

        let config = Config::new(url);
        let chain = Chain::fork_with_transport(config, transport.clone(), 1).unwrap();

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

        let config = Config::new(url);
        let chain = Chain::fork_with_transport(config, transport.clone(), 20_000_000).unwrap();
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

        let config = Config::new(url);
        let chain = Chain::fork_with_transport(config, transport.clone(), 9_000_000).unwrap();
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

        let config = Config::new(url);
        let mut chain = Chain::fork_with_transport(config, transport.clone(), 1).unwrap();

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

        let config = Config::new(url);
        let chain = Chain::fork_with_transport(config, transport.clone(), 1).unwrap();
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

        let config = Config::new(url);
        let chain = Chain::fork_with_transport(config, transport.clone(), 1).unwrap();
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

        let config = Config::new(url);
        let chain = Chain::fork_with_transport(config, transport.clone(), 1).unwrap();
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

        let config = Config::new(url);
        let result = Chain::fork_with_transport(config, transport.clone(), 100);
        assert!(
            result.is_err(),
            "Chain::fork must reject a block whose number does not match the requested height"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("100"),
            "error message must mention the requested block number: {err_msg}"
        );
        assert!(
            err_msg.contains("1"),
            "error message must mention the returned block number: {err_msg}"
        );
    }
}
