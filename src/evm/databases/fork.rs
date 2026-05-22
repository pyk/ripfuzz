//! Forked remote database for the EVM chain.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use alloy_primitives::{Address, B256, U256};
use anyhow::{Context as _, Result};
use revm::{
    DatabaseRef,
    bytecode::Bytecode,
    context::{BlockEnv, CfgEnv},
    context_interface::block::BlobExcessGasAndPrice,
    database::CacheDB,
    database_interface::DBErrorMarker,
    primitives::Bytes,
    state::AccountInfo,
};

use crate::evm::chain::{Chain, DEFAULT_DEPLOYER};
use crate::rpc_v2::Client;
use crate::vm::VM_ADDRESS;

/// Thin newtype around `anyhow::Error` so we can implement `DBErrorMarker`.
#[derive(Debug)]
pub struct ForkDBError(anyhow::Error);

impl std::fmt::Display for ForkDBError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ForkDBError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl DBErrorMarker for ForkDBError {}

impl From<anyhow::Error> for ForkDBError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}

/// Remote backend that satisfies [`DatabaseRef`].
///
/// All state caching is delegated to the RPC layer; this struct only maps
/// revm database operations to typed RPC calls.
#[derive(Clone, Debug)]
pub struct ForkDB {
    client: Arc<Client>,
    block_number: u64,
    /// Caches bytecode by code hash. RwLock chosen because `code_by_hash_ref`
    /// is a read-heavy while writes only happen on cache misses during
    /// `basic_ref`.
    contracts: Arc<RwLock<HashMap<B256, Bytecode>>>,
}

impl ForkDB {
    pub fn new(client: Arc<Client>, block_number: u64) -> Self {
        Self {
            client,
            block_number,
            contracts: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl DatabaseRef for ForkDB {
    type Error = ForkDBError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let (balance, nonce, code) = self
            .client
            .get_account(address, self.block_number)
            .map_err(ForkDBError::from)?;
        let bytecode = if code.is_empty() {
            Bytecode::default()
        } else {
            Bytecode::new_raw(code)
        };
        let code_hash = bytecode.hash_slow();
        if !bytecode.is_empty() {
            self.contracts
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(code_hash, bytecode.clone());
        }
        Ok(Some(AccountInfo {
            balance,
            nonce,
            code_hash,
            code: Some(bytecode),
            account_id: None,
        }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        if code_hash == revm::primitives::KECCAK_EMPTY || code_hash.is_zero() {
            return Ok(Bytecode::default());
        }
        match self
            .contracts
            .read()
            // TODO(pyk): handle lock poisoning here
            .unwrap_or_else(|e| e.into_inner())
            .get(&code_hash)
        {
            Some(code) => Ok(code.clone()),
            None => Err(ForkDBError::from(anyhow::anyhow!(
                "code hash {} not found in fork database",
                code_hash
            ))),
        }
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.client
            .get_storage_at(address, index, self.block_number)
            .map_err(ForkDBError::from)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let block = self
            .client
            .get_block_by_number(number)
            .map_err(ForkDBError::from)?;
        Ok(block.hash.unwrap_or_default())
    }
}

/// Configuration for a forked chain.
#[derive(Debug, Clone)]
pub struct ForkConfig {
    pub client: Arc<Client>,
    pub block_number: u64,
}

impl Chain<CacheDB<ForkDB>> {
    /// Create a new forked EVM pinned to a remote block.
    pub fn fork(config: ForkConfig) -> Result<Self> {
        let (chain_id, block) = config
            .client
            .get_fork_info(config.block_number)
            .with_context(|| format!("fetching fork info for block {}", config.block_number))?;

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

        let client = Arc::clone(&config.client);
        let fork_db = ForkDB::new(client, config.block_number);
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
            database: Some(database),
            block_env,
            cfg_env,
            deployer: DEFAULT_DEPLOYER,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use alloy_primitives::{B256, U256, address};
    use revm::primitives::Bytes;
    use revm::primitives::hardfork::SpecId;
    use serde_json::json;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER};
    use crate::rpc_v2::{Client, Config, MockTransport};
    use crate::vm::VM_ADDRESS;

    /// Chain::fork must seed the default deployer with U256::MAX balance,
    /// just like Chain::new, so that setup and deployment transactions
    /// never fail due to insufficient funds.
    #[test]
    fn chain_fork_seeds_deployer_with_max_balance() {
        let transport = MockTransport::default();

        let batch_payload = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]},
        ]);
        transport.mock_response(
            "mock://test",
            &batch_payload,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x1"},
                {"jsonrpc":"2.0","id":101,"result":{
                    "number":"0x1",
                    "timestamp":"0x1",
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0xffffffffffffffff",
                    "baseFeePerGas":"0x0",
                    "difficulty":"0x0",
                    "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
                }},
            ]),
        );

        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport.clone());
        let fork_config = ForkConfig {
            client: Arc::new(client),
            block_number: 1,
        };

        let chain = Chain::fork(fork_config).unwrap();
        assert_eq!(chain.deployer(), DEFAULT_DEPLOYER);

        // Chain::fork seeds the deployer locally, so only the batch should
        // have been fetched over RPC.
        assert_eq!(
            transport.call_count("mock://test", &batch_payload),
            1,
            "Chain::fork must fetch exactly one batch request"
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

        let batch_payload = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]},
        ]);
        transport.mock_response(
            "mock://test",
            &batch_payload,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x1"},
                {"jsonrpc":"2.0","id":101,"result":{
                    "number":"0x1",
                    "timestamp":"0x1",
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0xffffffffffffffff",
                    "baseFeePerGas":"0x0",
                    "difficulty":"0x0",
                    "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
                }},
            ]),
        );

        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport.clone());
        let fork_config = ForkConfig {
            client: Arc::new(client),
            block_number: 1,
        };

        let chain = Chain::fork(fork_config).unwrap();

        // Chain::fork injects both deployer and VM_ADDRESS locally, so only
        // the batch should have been fetched over RPC.
        assert_eq!(
            transport.call_count("mock://test", &batch_payload),
            1,
            "Chain::fork must fetch exactly one batch request"
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
    /// block number and timestamp instead of hardcoding AMSTERDAM.  A
    /// mainnet block past Cancun activation must use SpecId::CANCUN.
    #[test]
    fn chain_fork_uses_correct_spec_for_mainnet_cancun() {
        let transport = MockTransport::default();

        let batch_payload = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getBlockByNumber","params":[json!("0x1312d00"), json!(false)]},
        ]);
        transport.mock_response(
            "mock://test",
            &batch_payload,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x1"},
                {"jsonrpc":"2.0","id":101,"result":{
                    "number":"0x1312d00",
                    "timestamp":"0x65f5e100",
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0xffffffffffffffff",
                    "baseFeePerGas":"0x0",
                    "difficulty":"0x0",
                    "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
                }},
            ]),
        );

        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport.clone());
        let fork_config = ForkConfig {
            client: Arc::new(client),
            block_number: 20_000_000,
        };

        let chain = Chain::fork(fork_config).unwrap();
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

        let batch_payload = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getBlockByNumber","params":[json!("0x895440"), json!(false)]},
        ]);
        transport.mock_response(
            "mock://test",
            &batch_payload,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x2105"},
                {"jsonrpc":"2.0","id":101,"result":{
                    "number":"0x895440",
                    "timestamp":"0x665fd100",
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0xffffffffffffffff",
                    "baseFeePerGas":"0x0",
                    "difficulty":"0x0",
                    "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
                }},
            ]),
        );

        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport.clone());
        let fork_config = ForkConfig {
            client: Arc::new(client),
            block_number: 9_000_000,
        };

        let chain = Chain::fork(fork_config).unwrap();
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

        let batch_payload = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]},
        ]);
        // Use a non-zero base fee to trigger the bug.
        transport.mock_response(
            "mock://test",
            &batch_payload,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x1"},
                {"jsonrpc":"2.0","id":101,"result":{
                    "number":"0x1",
                    "timestamp":"0x1",
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0xffffffffffffffff",
                    "baseFeePerGas":"0x1",
                    "difficulty":"0x0",
                    "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
                }},
            ]),
        );

        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport.clone());
        let fork_config = ForkConfig {
            client: Arc::new(client),
            block_number: 1,
        };

        let mut chain = Chain::fork(fork_config).unwrap();

        assert_eq!(
            chain.block_env().basefee,
            0,
            "Chain::fork must zero basefee so gas_price=0 transactions validate"
        );

        // Execute a CALL against the locally-injected VM_ADDRESS using the
        // default gas_price of 0.  If basefee were kept non-zero this would
        // fail with GasPriceLessThanBasefee.
        // We also seed the beneficiary (Address::ZERO) so the fork DB is not
        // hit for block rewards during the transact.
        chain.seed_account(Address::ZERO, U256::ZERO).unwrap();
        let tx_result = chain
            .call(DEFAULT_DEPLOYER, VM_ADDRESS, U256::ZERO, Bytes::new())
            .unwrap();
        assert!(
            tx_result.success,
            "call with gas_price=0 must succeed in fork mode"
        );
    }

    /// Regression: ForkDB::code_by_hash_ref must resolve code that was
    /// previously fetched via basic_ref. Returning empty bytecode for every
    /// hash silently corrupts execution when revm or CacheDB falls back to
    /// the underlying database.
    #[test]
    fn forkdb_code_by_hash_ref_resolves_known_code() {
        let transport = MockTransport::default();
        let address = address!("0x0000000000000000000000000000000000000001");
        let block_tag = json!("0x1");

        // Mock the batch request that get_account sends.
        let batch_payload = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_getBalance","params":[json!(format!("0x{address:x}")), block_tag.clone()]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getTransactionCount","params":[json!(format!("0x{address:x}")), block_tag.clone()]},
            {"jsonrpc":"2.0","id":102,"method":"eth_getCode","params":[json!(format!("0x{address:x}")), block_tag]},
        ]);
        let code_hex = "0x600160005260016000f3";
        transport.mock_response(
            "mock://test",
            &batch_payload,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x0"},
                {"jsonrpc":"2.0","id":101,"result":"0x0"},
                {"jsonrpc":"2.0","id":102,"result": code_hex},
            ]),
        );

        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport.clone());
        let fork_db = ForkDB::new(Arc::new(client), 1);

        let info = fork_db.basic_ref(address).unwrap().unwrap();
        let code_hash = info.code_hash;
        let expected_code = info.code.unwrap();

        let resolved = fork_db.code_by_hash_ref(code_hash).unwrap();
        assert_eq!(
            resolved, expected_code,
            "ForkDB::code_by_hash_ref must return the same bytecode that basic_ref provided"
        );
    }

    /// Regression: Chain::fork must cache the fork block hash so that the
    /// BLOCKHASH opcode for the fork block number resolves locally instead of
    /// triggering an eth_getBlockByNumber RPC call.
    #[test]
    fn chain_fork_caches_fork_block_hash() {
        let transport = MockTransport::default();

        let fork_hash = B256::from([0xab; 32]);
        let batch_payload = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]},
        ]);
        transport.mock_response(
            "mock://test",
            &batch_payload,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x1"},
                {"jsonrpc":"2.0","id":101,"result":{
                    "number":"0x1",
                    "timestamp":"0x1",
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0xffffffffffffffff",
                    "baseFeePerGas":"0x0",
                    "difficulty":"0x0",
                    "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash": fork_hash
                }},
            ]),
        );

        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport.clone());
        let fork_config = ForkConfig {
            client: Arc::new(client),
            block_number: 1,
        };

        let chain = Chain::fork(fork_config).unwrap();
        let db = chain.database().unwrap();

        // This must resolve from cache and return the exact fork hash.
        let hash = db.block_hash_ref(1).unwrap();
        assert_eq!(hash, fork_hash, "fork block hash must be cached");

        // No extra eth_getBlockByNumber call should have been made.
        let extra_payload = json!([
            {"jsonrpc":"2.0","id":1,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]},
        ]);
        assert_eq!(
            transport.call_count("mock://test", &extra_payload),
            0,
            "block_hash_ref must not trigger an extra RPC for the fork block"
        );
    }

    /// Regression: Chain::fork must derive the blob base fee update fraction from
    /// the resolved `SpecId` instead of hardcoding the Cancun mainnet value
    /// (`3338477`).  Forking a Prague block on mainnet should use the Prague
    /// fraction (`5007716`), which produces a different blob gasprice for the same
    /// excess blob gas.
    #[test]
    fn chain_fork_uses_spec_aware_blob_fraction() {
        let transport = MockTransport::default();

        let batch_payload = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]},
        ]);
        transport.mock_response(
            "mock://test",
            &batch_payload,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x1"},
                {"jsonrpc":"2.0","id":101,"result":{
                    "number":"0x1",
                    "timestamp":"0x681b3057",
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0xffffffffffffffff",
                    "baseFeePerGas":"0x0",
                    "difficulty":"0x0",
                    "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "excessBlobGas":"0x4c4b40"
                }},
            ]),
        );

        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport.clone());
        let fork_config = ForkConfig {
            client: Arc::new(client),
            block_number: 1,
        };

        let chain = Chain::fork(fork_config).unwrap();
        let blob_info = chain.block_env().blob_excess_gas_and_price.unwrap();

        // Prague update fraction (5_007_716) yields blob_gasprice=2 for
        // excess_blob_gas=5_000_000.  The old hardcoded Cancun fraction
        // (3_338_477) would yield 4.
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

        let batch_payload = json!([
            {"jsonrpc":"2.0","id":100,"method":"eth_chainId","params":[]},
            {"jsonrpc":"2.0","id":101,"method":"eth_getBlockByNumber","params":[json!("0x1"), json!(false)]},
        ]);
        transport.mock_response(
            "mock://test",
            &batch_payload,
            json!([
                {"jsonrpc":"2.0","id":100,"result":"0x1"},
                {"jsonrpc":"2.0","id":101,"result":{
                    "number":"0x1",
                    "timestamp":"0x1",
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0xffffffffffffffff",
                    "baseFeePerGas":"0x0",
                    "difficulty":"0x0",
                    "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
                }},
            ]),
        );

        let config = Config::new("mock://test");
        let client = Client::new_with_transport(config, transport.clone());
        let fork_config = ForkConfig {
            client: Arc::new(client),
            block_number: 1,
        };

        let chain = Chain::fork(fork_config).unwrap();
        assert_eq!(
            chain.cfg_env().limit_contract_initcode_size,
            Some(usize::MAX),
            "Chain::fork must disable initcode size limit just like Chain::new"
        );
    }
}
