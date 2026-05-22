//! EVM chain state and executor.

use std::sync::Arc;

use alloy_primitives::{Address, B256, U256, address};
use anyhow::Context as _;
use revm::{
    Database, DatabaseCommit, DatabaseRef, MainBuilder, MainContext,
    bytecode::Bytecode,
    context::{BlockEnv, CfgEnv, Context, TxEnv},
    database::{CacheDB, InMemoryDB},
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
// ForkDb
// -----------------------------------------------------------------------------

/// Thin newtype around `anyhow::Error` so we can implement `DBErrorMarker`.
#[derive(Debug)]
pub struct ForkDbError(anyhow::Error);

impl std::fmt::Display for ForkDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ForkDbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl DBErrorMarker for ForkDbError {}

impl From<anyhow::Error> for ForkDbError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}

/// Remote backend that satisfies [`DatabaseRef`].
///
/// All state caching is delegated to the RPC layer; this struct only maps
/// revm database operations to typed RPC calls.
#[derive(Clone, Debug)]
pub struct ForkDb {
    client: Arc<Client>,
    block_number: u64,
}

impl ForkDb {
    pub fn new(client: Arc<Client>, block_number: u64) -> Self {
        Self {
            client,
            block_number,
        }
    }
}

impl DatabaseRef for ForkDb {
    type Error = ForkDbError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let (balance, nonce, code) = self
            .client
            .get_account(address, self.block_number)
            .map_err(ForkDbError::from)?;
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
            .map_err(ForkDbError::from)
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        let block = self
            .client
            .get_block_by_number(number)
            .map_err(ForkDbError::from)?;
        Ok(block.hash.unwrap_or_default())
    }
}

// -----------------------------------------------------------------------------
// ChainError
// -----------------------------------------------------------------------------

/// Error type for [`Chain`] operations.
#[derive(Debug)]
pub struct ChainError(anyhow::Error);

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ChainError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

impl From<anyhow::Error> for ChainError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}

// -----------------------------------------------------------------------------
// ForkConfig
// -----------------------------------------------------------------------------

/// Configuration for a forked chain.
#[derive(Debug, Clone)]
pub struct ForkConfig {
    pub client: Arc<Client>,
    pub block_number: u64,
}

// -----------------------------------------------------------------------------
// Chain
// -----------------------------------------------------------------------------

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
    block_env: BlockEnv,
    cfg_env: CfgEnv,
    deployer: Address,
}

impl Default for Chain<InMemoryDB> {
    fn default() -> Self {
        Self::new()
    }
}

impl Chain<InMemoryDB> {
    /// Create a new local sandbox EVM.
    pub fn new() -> Self {
        let mut block_env = BlockEnv {
            number: U256::from(1),
            beneficiary: Address::ZERO,
            timestamp: U256::from(1),
            gas_limit: u64::MAX,
            basefee: 0,
            difficulty: U256::ZERO,
            prevrandao: Some(B256::ZERO),
            blob_excess_gas_and_price: None,
            slot_num: 0,
        };
        block_env.set_blob_excess_gas_and_price(0, 3338477);

        let mut cfg_env = CfgEnv::default();
        cfg_env.chain_id = 1;
        cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        cfg_env.disable_nonce_check = true;
        cfg_env.disable_eip3607 = true;
        cfg_env.limit_contract_code_size = Some(usize::MAX);
        cfg_env.set_spec_and_mainnet_gas_params(SpecId::AMSTERDAM);

        let mut db = InMemoryDB::default();
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

impl Chain<CacheDB<ForkDb>> {
    /// Create a new forked EVM pinned to a remote block.
    pub fn fork(config: ForkConfig) -> Result<Self, ChainError> {
        let block = config
            .client
            .get_block_by_number(config.block_number)
            .with_context(|| format!("fetching block {}", config.block_number))?;

        let client = Arc::clone(&config.client);
        let fork_db = ForkDb::new(client, config.block_number);
        let database = CacheDB::new(fork_db);
        let chain_id = config.client.chain_id();

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

        let mut cfg_env = CfgEnv::default();
        cfg_env.chain_id = chain_id;
        cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        cfg_env.disable_nonce_check = true;
        cfg_env.disable_eip3607 = true;
        cfg_env.limit_contract_code_size = Some(usize::MAX);
        cfg_env.set_spec_and_mainnet_gas_params(SpecId::AMSTERDAM);

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
    pub fn seed_account(&mut self, address: Address, balance: U256) -> Result<(), ChainError> {
        let info = AccountInfo {
            balance,
            nonce: 0,
            code_hash: revm::primitives::KECCAK_EMPTY,
            code: None,
            account_id: None,
        };
        self.database
            .as_mut()
            .ok_or_else(|| ChainError::from(anyhow::anyhow!("database unavailable")))?
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
    ) -> Result<(Address, TransactionResult), ChainError> {
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
            .ok_or_else(|| ChainError::from(anyhow::anyhow!("create succeeded but no address")))?;
        Ok((address, result))
    }

    /// Execute a CALL against the given target.
    pub fn call(
        &mut self,
        caller: Address,
        target: Address,
        value: U256,
        data: Bytes,
    ) -> Result<TransactionResult, ChainError> {
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
    pub fn transact(&mut self, tx: TxEnv) -> Result<TransactionResult, ChainError> {
        let db = self
            .database
            .take()
            .ok_or_else(|| ChainError::from(anyhow::anyhow!("database unavailable")))?;
        let mut ctx = Context::mainnet().with_db(db);
        ctx.block = self.block_env.clone();
        ctx.cfg = self.cfg_env.clone();
        let mut evm = ctx.build_mainnet();
        let result = evm
            .transact_commit(tx)
            .map_err(|e| ChainError::from(anyhow::anyhow!("{e}")))?;
        self.database = Some(evm.ctx.journaled_state.database);
        Ok(TransactionResult::from(result))
    }

    /// Execute a raw transaction with an inspector and commit state changes.
    ///
    /// Returns the transaction result and the owned inspector so the caller can
    /// extract collected data (e.g. traces, coverage).
    pub fn inspect<INSP>(
        &mut self,
        tx: TxEnv,
        inspector: INSP,
    ) -> Result<(TransactionResult, INSP), ChainError>
    where
        INSP: Inspector<Context<BlockEnv, TxEnv, CfgEnv, D, revm::Journal<D>>>,
    {
        let db = self
            .database
            .take()
            .ok_or_else(|| ChainError::from(anyhow::anyhow!("database unavailable")))?;
        let mut ctx = Context::mainnet().with_db(db);
        ctx.block = self.block_env.clone();
        ctx.cfg = self.cfg_env.clone();
        let mut evm = ctx.build_mainnet_with_inspector(inspector);
        let result = evm
            .inspect_tx_commit(tx)
            .map_err(|e| ChainError::from(anyhow::anyhow!("{e}")))?;
        self.database = Some(evm.ctx.journaled_state.database);
        Ok((TransactionResult::from(result), evm.inspector))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::utils::keccak256;
    use revm::bytecode::opcode::{CODECOPY, MSTORE, PUSH1, PUSH2, RETURN};
    use revm::primitives::hardfork::SpecId;

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
    fn chain_new_seeds_deployer_with_max_balance() -> Result<(), ChainError> {
        let chain = Chain::new();
        assert_eq!(
            chain.deployer(),
            DEFAULT_DEPLOYER,
            "deployer should default to DEFAULT_DEPLOYER"
        );
        let db = chain
            .database()
            .ok_or_else(|| ChainError::from(anyhow::anyhow!("database unavailable")))?;
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
    fn chain_new_allows_contract_as_caller() -> Result<(), ChainError> {
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
    fn chain_new_injects_vm_address() -> Result<(), ChainError> {
        let chain = Chain::new();
        let db = chain
            .database()
            .ok_or_else(|| ChainError::from(anyhow::anyhow!("database unavailable")))?;
        let Ok(info) = db.basic_ref(VM_ADDRESS);
        let info =
            info.ok_or_else(|| ChainError::from(anyhow::anyhow!("VM_ADDRESS account missing")))?;
        let code = info
            .code
            .as_ref()
            .ok_or_else(|| ChainError::from(anyhow::anyhow!("VM_ADDRESS code missing")))?;
        assert!(
            !code.is_empty(),
            "Chain::new must inject non-empty code at VM_ADDRESS so extcodesize checks pass"
        );
        Ok(())
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
}
