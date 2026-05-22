//! EVM chain state and executor.

use alloy_primitives::{Address, U256, address};
use anyhow::{Context as _, Result};
use revm::{
    Database, DatabaseCommit, DatabaseRef, MainBuilder, MainContext,
    context::{BlockEnv, CfgEnv, Context, TxEnv},
    database::CacheDB,
    handler::ExecuteCommitEvm,
    inspector::{InspectCommitEvm, Inspector},
    primitives::{Bytes, TxKind},
    state::AccountInfo,
};

use crate::evm::result::TransactionResult;

/// Default deployer address: `address(uint160(uint256(keccak256("raptor deployer"))))`.
pub const DEFAULT_DEPLOYER: Address = address!("0xc34296175b9e78f66edbeaeb7acea4c615c092e1");

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
    pub database: Option<D>,
    pub cfg_env: CfgEnv,
    pub block_env: BlockEnv,
    pub deployer: Address,
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
