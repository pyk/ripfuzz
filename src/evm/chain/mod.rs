//! EVM chain state and executor.

use alloy_primitives::{Address, U256, address};
use anyhow::{Context as _, Result};
use revm::{
    MainBuilder, MainContext,
    context::{BlockEnv, CfgEnv, Context, TxEnv},
    handler::ExecuteCommitEvm,
    inspector::{InspectCommitEvm, Inspector},
    primitives::{Bytes, TxKind},
    state::AccountInfo,
};

use crate::evm::cheatcode::inspector::CheatcodeInspector;
use crate::evm::database::Database;
use crate::evm::result::TransactionResult;
use crate::evm::trace::{Inspector as TraceInspector, Trace};

/// Default deployer address: `address(uint160(uint256(keccak256("raptor deployer"))))`.
pub const DEFAULT_DEPLOYER: Address = address!("0xc34296175b9e78f66edbeaeb7acea4c615c092e1");

/// Configuration for a contract deployment.
#[derive(Debug, Clone)]
pub struct DeployOptions {
    pub caller: Address,
    pub value: U256,
    pub initcode: Bytes,
    pub gas_limit: u64,
}

impl DeployOptions {
    /// Create [`DeployOptions`] with the given initcode.
    ///
    /// Caller defaults to [`DEFAULT_DEPLOYER`]; override with [`Self::caller`].
    pub fn new(initcode: Bytes) -> Self {
        Self {
            caller: DEFAULT_DEPLOYER,
            value: U256::ZERO,
            initcode,
            gas_limit: u64::MAX,
        }
    }

    /// Set the account address used to deploy the contract.
    pub fn caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    /// Set the wei value sent with the deployment transaction.
    pub fn value(mut self, value: U256) -> Self {
        self.value = value;
        self
    }

    /// Set the gas limit for the deployment transaction.
    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self
    }
}

/// Result of a contract deployment, including the trace.
///
/// `address` is `None` when the constructor reverts or halts, but `result`
/// and `trace` are still populated so the caller can inspect the failure.
#[derive(Debug, Clone)]
pub struct Deployment {
    pub address: Option<Address>,
    pub result: TransactionResult,
    pub trace: Trace,
}

/// EVM Chain state and executor.
///
/// Owns EVM state ([`BlockEnv`](revm::context::BlockEnv),
/// [`CfgEnv`](revm::context::CfgEnv), and a [`Database`]).
///
/// Cloning a [`Chain`] produces an independent snapshot of state suitable for
/// isolated fuzzing runs.
#[derive(Clone, Debug)]
pub struct Chain {
    pub database: Option<Database>,
    pub cfg_env: CfgEnv,
    pub block_env: BlockEnv,
    pub deployer: Address,
}

impl Chain {
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
    pub fn database_mut(&mut self) -> Option<&mut Database> {
        self.database.as_mut()
    }

    /// Immutable access to the underlying database.
    ///
    /// Returns `None` if called while a transaction is in flight.
    pub fn database(&self) -> Option<&Database> {
        self.database.as_ref()
    }

    /// Deploy a contract and return the full [`Deployment`] result.
    ///
    /// A [`CheatcodeInspector`] is included so that target contracts can call
    /// raptor cheatcodes (e.g. `vm.warp`) during constructor execution.
    pub fn deploy(&mut self, opts: DeployOptions) -> Result<Deployment> {
        let inspector = (TraceInspector::new(), CheatcodeInspector::new());
        let tx = TxEnv {
            caller: opts.caller,
            kind: TxKind::Create,
            data: opts.initcode,
            gas_limit: opts.gas_limit,
            value: opts.value,
            ..Default::default()
        };
        let (result, (trace_inspector, _)) = self.inspect(tx, inspector)?;
        let address = result.created_address;
        let trace = trace_inspector.into_trace();
        Ok(Deployment {
            address,
            result,
            trace,
        })
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
        INSP: Inspector<Context<BlockEnv, TxEnv, CfgEnv, Database, revm::Journal<Database>>>,
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

mod empty;
mod fork;
