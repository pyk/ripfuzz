//! EVM chain state and executor.

use std::convert::Infallible;
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256, address};
use anyhow::{Context as _, Result};
use revm::{
    Database, DatabaseCommit, DatabaseRef, MainBuilder, MainContext,
    bytecode::Bytecode,
    context::{BlockEnv, CfgEnv, Context, TxEnv},
    database::{CacheDB, EmptyDBTyped},
    database_interface::DBErrorMarker,
    handler::ExecuteCommitEvm,
    inspector::{InspectCommitEvm, Inspector},
    primitives::hardfork::SpecId,
    primitives::{Bytes, TxKind},
    state::AccountInfo,
};

use crate::evm::result::TransactionResult;
use crate::rpc_v2::Client;

/// Default deployer address: `address(uint160(uint256(keccak256("raptor deployer"))))`.
pub const DEFAULT_DEPLOYER: Address = address!("0xc34296175b9e78f66edbeaeb7acea4c615c092e1");

// -----------------------------------------------------------------------------
// ForkDB
// -----------------------------------------------------------------------------

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
}

impl ForkDB {
    pub fn new(client: Arc<Client>, block_number: u64) -> Self {
        Self {
            client,
            block_number,
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
        Ok(Some(AccountInfo {
            balance,
            nonce,
            code_hash,
            code: Some(bytecode),
            account_id: None,
        }))
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(Bytecode::default())
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

// ----------------------------------------------------------------------------
// ForkConfig
// ----------------------------------------------------------------------------

/// Configuration for a forked chain.
#[derive(Debug, Clone)]
pub struct ForkConfig {
    pub client: Arc<Client>,
    pub block_number: u64,
}

// ----------------------------------------------------------------------------
// LocalDB
// ----------------------------------------------------------------------------

/// Wrapper around `revm::EmptyDB` that returns `Some(AccountInfo::default())`
/// for every address so that `CacheDB` never marks an account as
/// `AccountState::NotExisting`.
///
/// In revm, `CacheDB` distinguishes between "non-existing" (`None`) and
/// "empty" (`Some(AccountInfo::default())`). If an account is marked as
/// `NotExisting`, state transitions differ when the account is later created
/// (e.g. via `deal` or `etch`). A sandbox fuzzer has no state trie, so every
/// address should be treated as empty rather than non-existing.
///
/// Foundry uses the same trick: see `foundry-evm-core::backend::EmptyDBWrapper`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalDB(EmptyDBTyped<Infallible>);

impl DatabaseRef for LocalDB {
    type Error = Infallible;

    fn basic_ref(&self, _address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(Some(AccountInfo::default()))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.0.code_by_hash_ref(code_hash)
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.0.storage_ref(address, index)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.0.block_hash_ref(number)
    }
}

// ----------------------------------------------------------------------------
// Chain
// ----------------------------------------------------------------------------

/// EVM Chain state and executor.
///
/// `D` is the database type. It must satisfy both [`Database`] and
/// [`DatabaseCommit`] so that revm can read state and write results back.
///
/// Cloning a [`Chain`] produces an independent snapshot of state suitable for
/// isolated fuzzing runs.
#[derive(Clone, Debug)]
pub struct Chain<D>
where
    D: Database + DatabaseCommit,
{
    database: Option<D>,
    cfg_env: CfgEnv,
    block_env: BlockEnv,
    deployer: Address,
}

impl Default for Chain<CacheDB<LocalDB>> {
    fn default() -> Self {
        Self::new()
    }
}

impl Chain<CacheDB<LocalDB>> {
    /// Create a new local sandbox EVM.
    pub fn new() -> Self {
        let mut cfg_env = CfgEnv::default();
        cfg_env.chain_id = 1;
        cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        cfg_env.disable_nonce_check = true;
        cfg_env.disable_eip3607 = true;
        cfg_env.limit_contract_code_size = Some(usize::MAX);
        cfg_env.limit_contract_initcode_size = Some(usize::MAX);
        cfg_env.set_spec_and_mainnet_gas_params(SpecId::AMSTERDAM);

        let mut block_env = BlockEnv {
            number: U256::from(1),
            beneficiary: Address::ZERO,
            timestamp: U256::from(1_438_269_988_u64),
            gas_limit: u64::MAX,
            basefee: 0,
            difficulty: U256::ZERO,
            prevrandao: Some(B256::ZERO),
            blob_excess_gas_and_price: None,
            slot_num: 0,
        };

        // NOTE: This is required for post-Cancun
        block_env.set_blob_excess_gas_and_price(0, 3338477);

        let mut db = CacheDB::new(LocalDB::default());
        let info = AccountInfo {
            balance: U256::MAX,
            nonce: 0,
            code_hash: revm::primitives::KECCAK_EMPTY,
            code: None,
            account_id: None,
        };
        db.insert_account_info(DEFAULT_DEPLOYER, info);

        // Insert a dummy VM contract so Solidity's `extcodesize` check passes
        // when a target calls raptor cheatcodes during deployment or setup.
        let vm_code = Bytecode::new_raw(Bytes::from_static(&[0x00]));
        db.insert_account_info(
            crate::vm::VM_ADDRESS,
            AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: vm_code.hash_slow(),
                code: Some(vm_code),
                account_id: None,
            },
        );

        Self {
            database: Some(db),
            block_env,
            cfg_env,
            deployer: DEFAULT_DEPLOYER,
        }
    }
}

impl Chain<CacheDB<ForkDB>> {
    /// Create a new forked EVM pinned to a remote block.
    pub fn fork(config: ForkConfig) -> Result<Self> {
        let chain_id = config.client.chain_id();

        let mut cfg_env = CfgEnv::default();
        cfg_env.chain_id = chain_id;
        cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        cfg_env.disable_nonce_check = true;
        cfg_env.disable_eip3607 = true;
        cfg_env.limit_contract_code_size = Some(usize::MAX);
        cfg_env.set_spec_and_mainnet_gas_params(SpecId::AMSTERDAM);

        let block = config
            .client
            .get_block_by_number(config.block_number)
            .with_context(|| format!("fetching block {}", config.block_number))?;

        let client = Arc::clone(&config.client);
        let fork_db = ForkDB::new(client, config.block_number);
        let mut database = CacheDB::new(fork_db);

        let mut block_env = BlockEnv {
            number: U256::from(block.number),
            beneficiary: block.coinbase,
            timestamp: U256::from(block.timestamp),
            gas_limit: block.gas_limit.to(),
            basefee: block.basefee.to(),
            difficulty: block.difficulty,
            prevrandao: block.prevrandao,
            blob_excess_gas_and_price: None,
            slot_num: 0,
        };
        if let Some(excess) = block.excess_blob_gas {
            block_env.set_blob_excess_gas_and_price(excess.to(), 3338477);
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

        Ok(Self {
            database: Some(database),
            block_env,
            cfg_env,
            deployer: DEFAULT_DEPLOYER,
        })
    }
}

impl<Inner: DatabaseRef> Chain<CacheDB<Inner>> {
    /// Seed an account with balance and zero nonce.
    pub fn seed_account(&mut self, address: Address, balance: U256) -> Result<()> {
        let info = AccountInfo {
            balance,
            nonce: 0,
            code_hash: revm::primitives::KECCAK_EMPTY,
            code: None,
            account_id: None,
        };
        self.database
            .as_mut()
            .context("database unavailable")?
            .insert_account_info(address, info);
        Ok(())
    }
}

impl<D> Chain<D>
where
    D: Database + DatabaseCommit,
{
    /// Mutable access to the block environment.
    pub fn block_env_mut(&mut self) -> &mut BlockEnv {
        &mut self.block_env
    }

    /// Immutable access to the block environment.
    pub fn block_env(&self) -> &BlockEnv {
        &self.block_env
    }

    /// Mutable access to the configuration environment.
    pub fn cfg_env_mut(&mut self) -> &mut CfgEnv {
        &mut self.cfg_env
    }

    /// Immutable access to the configuration environment.
    pub fn cfg_env(&self) -> &CfgEnv {
        &self.cfg_env
    }

    /// Immutable access to the deployer address.
    pub fn deployer(&self) -> Address {
        self.deployer
    }

    /// Mutable access to the underlying database.
    ///
    /// Returns `None` if called while a transaction is in flight (the database
    /// is temporarily moved into revm during execution).
    pub fn database_mut(&mut self) -> Option<&mut D> {
        self.database.as_mut()
    }

    /// Immutable access to the underlying database.
    ///
    /// Returns `None` if called while a transaction is in flight.
    pub fn database(&self) -> Option<&D> {
        self.database.as_ref()
    }

    /// Deploy a contract and return the deployed address + result.
    pub fn deploy(
        &mut self,
        caller: Address,
        value: U256,
        initcode: Bytes,
    ) -> Result<(Address, TransactionResult)> {
        let tx = TxEnv {
            caller,
            kind: TxKind::Create,
            data: initcode,
            gas_limit: u64::MAX,
            value,
            ..Default::default()
        };
        let result = self.transact(tx)?;
        let address = result
            .created_address
            .context("create succeeded but no address")?;
        Ok((address, result))
    }

    /// Execute a CALL against the given target.
    pub fn call(
        &mut self,
        caller: Address,
        target: Address,
        value: U256,
        data: Bytes,
    ) -> Result<TransactionResult> {
        let tx = TxEnv {
            caller,
            kind: TxKind::Call(target),
            data,
            gas_limit: u64::MAX,
            value,
            ..Default::default()
        };
        self.transact(tx)
    }

    /// Execute a raw transaction and commit state changes.
    pub fn transact(&mut self, tx: TxEnv) -> Result<TransactionResult> {
        let db = self.database.take().context("database unavailable")?;
        let mut ctx = Context::mainnet().with_db(db);
        ctx.block = self.block_env.clone();
        ctx.cfg = self.cfg_env.clone();
        let mut evm = ctx.build_mainnet();
        let result = evm.transact_commit(tx).context("transact_commit failed")?;
        self.database = Some(evm.ctx.journaled_state.database);
        Ok(TransactionResult::from(result))
    }

    /// Execute a raw transaction with an inspector and commit state changes.
    ///
    /// Returns the transaction result and the owned inspector so the caller can
    /// extract collected data (e.g. traces, coverage).
    pub fn inspect<INSP>(&mut self, tx: TxEnv, inspector: INSP) -> Result<(TransactionResult, INSP)>
    where
        INSP: Inspector<Context<BlockEnv, TxEnv, CfgEnv, D, revm::Journal<D>>>,
    {
        let db = self.database.take().context("database unavailable")?;
        let mut ctx = Context::mainnet().with_db(db);
        ctx.block = self.block_env.clone();
        ctx.cfg = self.cfg_env.clone();
        let mut evm = ctx.build_mainnet_with_inspector(inspector);
        let result = evm
            .inspect_tx_commit(tx)
            .context("revm transaction failed")?;
        self.database = Some(evm.ctx.journaled_state.database);
        Ok((TransactionResult::from(result), evm.inspector))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use alloy_primitives::utils::keccak256;
    use revm::bytecode::opcode::{CODECOPY, MSTORE, PUSH1, PUSH2, RETURN};
    use revm::primitives::hardfork::SpecId;
    use serde_json::json;

    use crate::rpc_v2::{Client, Config, MockTransport};
    use crate::vm::VM_ADDRESS;

    #[test]
    fn chain_new_uses_latest_spec() {
        let chain = Chain::new();
        assert_eq!(
            chain.cfg_env().spec,
            SpecId::AMSTERDAM,
            "Chain::new should use latest spec (AMSTERDAM)"
        );
    }

    #[test]
    fn default_deployer_matches_raptor_deployer_string() {
        let hash = keccak256(b"raptor deployer");
        let expected = revm::primitives::Address::from_word(hash);
        assert_eq!(expected, DEFAULT_DEPLOYER);
    }

    #[test]
    fn chain_new_seeds_deployer_with_max_balance() -> Result<()> {
        let chain = Chain::new();
        assert_eq!(
            chain.deployer(),
            DEFAULT_DEPLOYER,
            "deployer should default to DEFAULT_DEPLOYER"
        );
        let db = chain.database().context("database unavailable")?;
        let Ok(info) = db.basic_ref(DEFAULT_DEPLOYER);
        let balance = info.map(|i| i.balance).unwrap_or_default();
        assert_eq!(
            balance,
            U256::MAX,
            "deployer must be seeded with U256::MAX in Chain::new"
        );
        Ok(())
    }

    #[test]
    fn chain_new_allows_contract_as_caller() -> Result<()> {
        let mut chain = Chain::new();

        // Initcode that returns 1 byte of runtime code (0x00 STOP) so the
        // deployed address has non-empty code.
        let initcode = Bytes::from_static(&[
            PUSH1, 0x01, // PUSH1 1
            PUSH1, 0x00,   // PUSH1 0
            MSTORE, // MSTORE
            PUSH1, 0x01, // PUSH1 1
            PUSH1, 0x00,   // PUSH1 0
            RETURN, // RETURN
        ]);

        let (deployed_address, _) = chain.deploy(DEFAULT_DEPLOYER, U256::ZERO, initcode)?;

        // Calling from a contract address should succeed when EIP-3607 is disabled.
        let result = chain.call(deployed_address, Address::ZERO, U256::ZERO, Bytes::new());
        assert!(
            result.is_ok(),
            "EIP-3607 must be disabled so a contract can act as caller"
        );
        Ok(())
    }

    /// Chain::new must inject a dummy contract at the raptor VM address so
    /// that Solidity `extcodesize` checks do not revert when a target contract
    /// calls cheatcodes during deployment or setup.
    #[test]
    fn chain_new_injects_vm_address() -> Result<()> {
        let chain = Chain::new();
        let db = chain.database().context("database unavailable")?;
        let Ok(info) = db.basic_ref(VM_ADDRESS);
        let info = info.context("VM_ADDRESS account missing")?;
        let code = info.code.as_ref().context("VM_ADDRESS code missing")?;
        assert!(
            !code.is_empty(),
            "Chain::new must inject non-empty code at VM_ADDRESS so extcodesize checks pass"
        );
        Ok(())
    }

    /// Chain::new must use a database that returns `Some(AccountInfo::default())`
    /// for never-seen addresses.  If `Database::basic` returns `None`,
    /// revm's `CacheDB` marks the account as `AccountState::NotExisting`.
    /// A sandbox has no state trie, so there is no concept of "non-existing"
    /// vs "empty"; every address must be treated as empty.
    #[test]
    fn chain_new_returns_default_account_info_for_unknown_address() {
        let mut chain = Chain::new();
        let db = chain.database_mut().expect("database should be available");
        let unknown = address!("0x00000000000000000000000000000000000000ab");
        let info = db.basic(unknown).unwrap();
        assert!(
            info.is_some(),
            "a sandbox database must return Some(AccountInfo::default()) for every address; \
             got None, which marks the account as NotExisting in CacheDB"
        );
        assert_eq!(info.unwrap(), AccountInfo::default());
    }

    /// Chain::new must use the Ethereum mainnet block 1 timestamp
    /// (`1438269988`) instead of a small sentinel like 1, which predates the
    /// Unix epoch and can trigger underflows in contracts that compare
    /// `block.timestamp` against deployment time or constant offsets.
    #[test]
    fn chain_new_uses_mainnet_block_one_timestamp() {
        let chain = Chain::new();
        assert_eq!(
            chain.block_env().timestamp,
            U256::from(1_438_269_988_u64),
            "Chain::new should use the mainnet block 1 timestamp (1438269988)"
        );
    }

    /// Chain::new must disable the contract code size limit so
    /// that large factory contracts or inlined targets can deploy.
    #[test]
    fn chain_new_allows_unlimited_contract_size() {
        let mut chain = Chain::new();

        // Build initcode that returns 0x8001 bytes (32769) of runtime code,
        // which is one byte larger than the EIP-7954 limit of 0x8000 (32768)
        // enforced for the AMSTERDAM spec.
        //
        // Initcode:
        //   PUSH2 0x8001       // size to copy
        //   PUSH1 0x0e         // offset in this initcode to padding
        //   PUSH1 0x00         // dest offset in memory
        //   CODECOPY           // copy padding into memory
        //   PUSH2 0x8001       // size to return
        //   PUSH1 0x00         // mem offset
        //   RETURN             // return memory as runtime code
        let mut initcode = vec![
            PUSH2, 0x80, 0x01, // PUSH2 0x8001
            PUSH1, 0x0e, // PUSH1 0x0e
            PUSH1, 0x00,     // PUSH1 0x00
            CODECOPY, // CODECOPY
            PUSH2, 0x80, 0x01, // PUSH2 0x8001
            PUSH1, 0x00,   // PUSH1 0x00
            RETURN, // RETURN
        ];
        initcode.extend(std::iter::repeat(0x00).take(0x8001));

        let result = chain.deploy(DEFAULT_DEPLOYER, U256::ZERO, Bytes::from(initcode));
        assert!(
            result.is_ok(),
            "Chain::new must disable code size limit so contracts > 32 KB can deploy"
        );

        let (address, tx_result) = result.unwrap();
        assert!(tx_result.success, "large deployment must succeed");
        assert_ne!(
            address,
            Address::ZERO,
            "must return a valid deployed address"
        );

        // Verify the deployed bytecode is actually 32769 bytes.
        let db = chain.database().expect("database should be available");
        let info = db
            .basic_ref(address)
            .unwrap()
            .expect("account should exist");
        let code_len = info.code.map(|c| c.len()).unwrap_or(0);
        assert_eq!(code_len, 0x8001, "deployed code must be 32769 bytes");
    }

    /// Chain::fork must seed the default deployer with U256::MAX balance,
    /// just like Chain::new, so that setup and deployment transactions
    /// never fail due to insufficient funds.
    #[test]
    fn chain_fork_seeds_deployer_with_max_balance() -> Result<()> {
        let transport = Arc::new(MockTransport::default());

        // Minimal block header for eth_getBlockByNumber.
        transport.insert(
            "eth_getBlockByNumber",
            &[json!("0x1"), json!(false)],
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "number": "0x1",
                    "timestamp": "0x1",
                    "miner": "0x0000000000000000000000000000000000000000",
                    "gasLimit": "0xffffffffffffffff",
                    "baseFeePerGas": "0x0",
                    "difficulty": "0x0",
                    "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash": "0x0000000000000000000000000000000000000000000000000000000000000000"
                }
            }),
        );

        // Remote state for the deployer: zero balance so the test
        // fails if Chain::fork forgets to override it locally.
        transport.insert(
            "eth_getBalance",
            &[
                json!("0xc34296175b9e78f66edbeaeb7acea4c615c092e1"),
                json!("0x1"),
            ],
            json!({"jsonrpc": "2.0", "id": 1, "result": "0x0"}),
        );
        transport.insert(
            "eth_getTransactionCount",
            &[
                json!("0xc34296175b9e78f66edbeaeb7acea4c615c092e1"),
                json!("0x1"),
            ],
            json!({"jsonrpc": "2.0", "id": 2, "result": "0x0"}),
        );
        transport.insert(
            "eth_getCode",
            &[
                json!("0xc34296175b9e78f66edbeaeb7acea4c615c092e1"),
                json!("0x1"),
            ],
            json!({"jsonrpc": "2.0", "id": 3, "result": "0x"}),
        );

        let config = Config::new().urls(vec!["mock://test".into()]).chain_id(1);
        let client = Client::new_with_transport(config, transport);
        let fork_config = ForkConfig {
            client: Arc::new(client),
            block_number: 1,
        };

        let chain = Chain::fork(fork_config)?;
        assert_eq!(chain.deployer(), DEFAULT_DEPLOYER);

        let db = chain.database().context("database unavailable")?;
        let info = db
            .basic_ref(DEFAULT_DEPLOYER)
            .context("revm transaction failed")?
            .context("deployer account missing")?;
        assert_eq!(
            info.balance,
            U256::MAX,
            "deployer must be seeded with U256::MAX in Chain::fork"
        );
        Ok(())
    }
}
