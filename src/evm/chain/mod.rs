//! EVM chain state and executor.

use alloy_primitives::{Address, U256, address};
use alloy_sol_types::SolCall;
use anyhow::{Context as _, Result};
use revm::{
    MainBuilder, MainContext,
    context::{BlockEnv, CfgEnv, Context, TxEnv},
    context_interface::either::Either,
    handler::ExecuteCommitEvm,
    inspector::{InspectCommitEvm, Inspector as RevmInspector, NoOpInspector},
    primitives::{Bytes, TxKind},
    state::AccountInfo,
};

pub use crate::evm::chain::config::Config;
use crate::evm::{cheatcode, coverage, database, result, trace};

pub mod config;
mod empty;
mod fork;

/// Default deployer address: `address(uint160(uint256(keccak256("raptor deployer"))))`.
pub const DEFAULT_DEPLOYER: Address = address!("0xc34296175b9e78f66edbeaeb7acea4c615c092e1");

/// Configuration for a contract deployment.
#[derive(Debug, Clone)]
pub struct DeployInput {
    pub caller: Address,
    pub value: U256,
    pub initcode: Bytes,
    pub gas_limit: u64,
}

impl DeployInput {
    /// Create [`DeployInput`] with the given initcode.
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
pub struct DeployOutput {
    pub address: Option<Address>,
    pub result: result::TransactionResult,
    pub trace: trace::Trace,
}

/// Result of a setup call, including the trace.
#[derive(Debug, Clone)]
pub struct SetupOutput {
    pub result: result::TransactionResult,
    pub trace: trace::Trace,
}

alloy_sol_types::sol! {
    interface Setup {
        function setup() external;
    }
}

/// Configuration for a setup call.
#[derive(Debug, Clone)]
pub struct SetupInput {
    pub caller: Address,
    pub target: Address,
    pub calldata: Bytes,
    pub value: U256,
    pub gas_limit: u64,
}

impl SetupInput {
    /// Create [`SetupInput`] for the given target with the default `setup()` selector.
    ///
    /// Caller defaults to [`DEFAULT_DEPLOYER`]; override with [`Self::caller`].
    pub fn new(target: Address) -> Self {
        Self {
            caller: DEFAULT_DEPLOYER,
            target,
            calldata: Bytes::from(Setup::setupCall::new(()).abi_encode()),
            value: U256::ZERO,
            gas_limit: u64::MAX,
        }
    }

    /// Set the calldata for the setup transaction.
    pub fn calldata(mut self, calldata: Bytes) -> Self {
        self.calldata = calldata;
        self
    }

    /// Set the account address used to send the setup transaction.
    pub fn caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    /// Set the wei value sent with the setup transaction.
    pub fn value(mut self, value: U256) -> Self {
        self.value = value;
        self
    }

    /// Set the gas limit for the setup transaction.
    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self
    }
}

/// A single CALL transaction to execute in a sequence.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub caller: Address,
    pub target: Address,
    pub calldata: Bytes,
    pub value: U256,
    pub gas_limit: u64,
}

impl Transaction {
    /// Create a [`Transaction`] for the given target.
    ///
    /// Caller defaults to [`DEFAULT_DEPLOYER`]; override with [`Self::caller`].
    /// Calldata defaults to empty bytes; override with [`Self::calldata`].
    pub fn new(target: Address) -> Self {
        Self {
            caller: DEFAULT_DEPLOYER,
            target,
            calldata: Bytes::new(),
            value: U256::ZERO,
            gas_limit: u64::MAX,
        }
    }

    /// Set the calldata for the transaction.
    pub fn calldata(mut self, calldata: Bytes) -> Self {
        self.calldata = calldata;
        self
    }

    /// Set the account address used to send the transaction.
    pub fn caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    /// Set the wei value sent with the transaction.
    pub fn value(mut self, value: U256) -> Self {
        self.value = value;
        self
    }

    /// Set the gas limit for the transaction.
    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self
    }
}

/// Options for executing a sequence of transactions.
#[derive(Debug, Clone)]
pub struct ExecInput {
    pub transactions: Vec<Transaction>,
}

impl ExecInput {
    /// Create an [`ExecInput`] with the given transactions.
    pub fn new(transactions: Vec<Transaction>) -> Self {
        Self { transactions }
    }
}

impl Default for ExecInput {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

/// Result of executing a sequence of transactions.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub results: Vec<result::TransactionResult>,
    pub trace: Option<trace::Trace>,
    pub coverage: Option<coverage::map::LocalCoverage>,
}

/// EVM Chain state and executor.
///
/// Owns EVM state ([`BlockEnv`](revm::context::BlockEnv),
/// [`CfgEnv`](revm::context::CfgEnv), and a [`database::Database`]).
///
/// Cloning a [`Chain`] produces an independent snapshot of state suitable for
/// isolated fuzzing runs.
#[derive(Clone, Debug)]
pub struct Chain {
    pub database: Option<database::Database>,
    pub cfg_env: CfgEnv,
    pub block_env: BlockEnv,
    pub deployer: Address,
    pub config: Config,
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
    pub fn database_mut(&mut self) -> Option<&mut database::Database> {
        self.database.as_mut()
    }

    /// Immutable access to the underlying database.
    ///
    /// Returns `None` if called while a transaction is in flight.
    pub fn database(&self) -> Option<&database::Database> {
        self.database.as_ref()
    }

    /// Create a new chain.
    ///
    /// When [`Config::fork`](super::Config) is `Some`, the chain is forked
    /// from a remote RPC node pinned to [`Config::fork_block_number`].
    /// Otherwise an empty sandbox chain is created.
    pub fn new(config: Config) -> Result<Self> {
        match config.fork.clone() {
            Some(fork_config) => {
                let agent_cfg = ureq::Agent::config_builder()
                    .timeout_global(Some(std::time::Duration::from_millis(
                        fork_config.timeout_ms,
                    )))
                    .build();
                let agent = ureq::Agent::new_with_config(agent_cfg);
                Self::fork_with_transport(fork_config, agent)
            }
            None => Ok(Self::empty(config)),
        }
    }

    /// Deploy a contract and return the full [`DeployOutput`] result.
    ///
    /// A [`cheatcode::Inspector`] is included so that target contracts can call
    /// raptor cheatcodes (e.g. `vm.warp`) during constructor execution.
    pub fn deploy(&mut self, opts: DeployInput) -> Result<DeployOutput> {
        let inspector = (
            trace::Inspector::new(),
            cheatcode::Inspector::new(self.config.cheatcode.clone()),
        );
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
        Ok(DeployOutput {
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
    ) -> Result<result::TransactionResult> {
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

    /// Execute a setup CALL against the given target and return the full result with trace.
    pub fn setup(&mut self, opts: SetupInput) -> Result<SetupOutput> {
        let inspector = (
            trace::Inspector::new(),
            cheatcode::Inspector::new(self.config.cheatcode.clone()),
        );
        let tx = TxEnv {
            caller: opts.caller,
            kind: TxKind::Call(opts.target),
            data: opts.calldata,
            gas_limit: opts.gas_limit,
            value: opts.value,
            ..Default::default()
        };
        let (result, (trace_inspector, _)) = self.inspect(tx, inspector)?;
        let trace = trace_inspector.into_trace();
        Ok(SetupOutput { result, trace })
    }

    /// Execute a sequence of transactions and commit state changes.
    ///
    /// The same inspector is reused across all transactions, so cheatcode
    /// effects (e.g. `vm.warp`) and coverage collection persist from one
    /// transaction to the next.
    pub fn exec(&mut self, input: ExecInput) -> Result<ExecOutput> {
        let inspector = (
            Either::Left::<cheatcode::Inspector, NoOpInspector>(cheatcode::Inspector::new(
                self.config.cheatcode.clone(),
            )),
            (
                if self.config.trace {
                    Either::Left(trace::Inspector::new())
                } else {
                    Either::Right(NoOpInspector)
                },
                if self.config.coverage {
                    Either::Left(coverage::Inspector::new())
                } else {
                    Either::Right(NoOpInspector)
                },
            ),
        );
        let mut results = Vec::with_capacity(input.transactions.len());

        let db = self.database.take().context("database unavailable")?;
        let mut ctx = Context::mainnet().with_db(db);
        ctx.block = self.block_env.clone();
        ctx.cfg = self.cfg_env.clone();
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        for tx in input.transactions {
            let tx_env = TxEnv {
                caller: tx.caller,
                kind: TxKind::Call(tx.target),
                data: tx.calldata,
                gas_limit: tx.gas_limit,
                value: tx.value,
                ..Default::default()
            };
            let result = evm
                .inspect_tx_commit(tx_env)
                .context("revm transaction failed")?;
            // TODO: refactor the result module, rename it to transaction.rs or something
            results.push(result::TransactionResult::from(result));
        }

        let inspector = evm.inspector;
        self.database = Some(evm.ctx.journaled_state.database);
        self.block_env = evm.ctx.block;
        self.cfg_env = evm.ctx.cfg;

        Ok(ExecOutput {
            results,
            trace: match inspector.1.0 {
                Either::Left(t) => Some(t.into_trace()),
                Either::Right(_) => None,
            },
            coverage: match inspector.1.1 {
                Either::Left(c) => Some(c.into_coverage()),
                Either::Right(_) => None,
            },
        })
    }

    /// Execute a raw transaction and commit state changes.
    pub fn transact(&mut self, tx: TxEnv) -> Result<result::TransactionResult> {
        let db = self.database.take().context("database unavailable")?;
        let mut ctx = Context::mainnet().with_db(db);
        ctx.block = self.block_env.clone();
        ctx.cfg = self.cfg_env.clone();
        let mut evm = ctx.build_mainnet();
        let result = evm.transact_commit(tx).context("transact_commit failed")?;
        self.database = Some(evm.ctx.journaled_state.database);
        self.block_env = evm.ctx.block;
        self.cfg_env = evm.ctx.cfg;
        Ok(result::TransactionResult::from(result))
    }

    /// Execute a raw transaction with an inspector and commit state changes.
    ///
    /// Returns the transaction result and the owned inspector so the caller can
    /// extract collected data (e.g. traces, coverage).
    pub fn inspect<INSP>(
        &mut self,
        tx: TxEnv,
        inspector: INSP,
    ) -> Result<(result::TransactionResult, INSP)>
    where
        INSP: RevmInspector<
            Context<BlockEnv, TxEnv, CfgEnv, database::Database, revm::Journal<database::Database>>,
        >,
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
        self.block_env = evm.ctx.block;
        self.cfg_env = evm.ctx.cfg;
        Ok((result::TransactionResult::from(result), evm.inspector))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::contract;
    use crate::evm::chain::{Chain, Config, DeployInput, ExecInput, SetupInput, Transaction};
    use crate::foundry;
    use crate::target::Contract;

    alloy_sol_types::sol! {
        interface WarpTarget {
            function getBlockTimestamp() external view returns (uint256);
            function setup() external;
            function actionWarp() external;
            function invariant_warp() external view;
        }
    }

    const EXPECTED_TIMESTAMP: U256 = U256::from_limbs([1_234_567_890, 0, 0, 0]);

    fn load_warp_fixture() -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-with-cheatcodes");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from("src/WarpTarget.sol:WarpTarget").unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    fn deploy_and_setup_warp() -> (Chain, Address) {
        let contract = load_warp_fixture();
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_opts = SetupInput::new(target);
        let setup = chain.setup(setup_opts).unwrap();
        assert!(setup.result.success, "setup must succeed");

        (chain, target)
    }

    /// A sequence of transactions where the first mutates EVM state via a
    /// cheatcode and the second observes the mutated state.
    #[test]
    fn execute_sequence_preserves_cheatcode_state() {
        let (mut chain, target) = deploy_and_setup_warp();

        // First transaction: warp timestamp back to EXPECTED_TIMESTAMP.
        // Second transaction: invariant checks that block.timestamp matches.
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                WarpTarget::actionWarpCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                WarpTarget::invariant_warpCall::new(()).abi_encode(),
            )),
        ];

        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results[0].success, "actionWarp must succeed");
        assert!(
            execution.results[1].success,
            "invariant must see warped timestamp"
        );
    }

    /// Coverage is collected across all transactions in a sequence.
    #[test]
    fn execute_with_coverage_collects_across_sequence() {
        let (mut chain, target) = deploy_and_setup_warp();
        chain.config.coverage = true;

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                WarpTarget::actionWarpCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                WarpTarget::getBlockTimestampCall::new(()).abi_encode(),
            )),
        ];

        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 2);
        assert!(execution.results.iter().all(|r| r.success));

        let coverage = execution.coverage.expect("coverage must be present");
        assert!(
            !coverage.contracts.is_empty(),
            "coverage should contain at least one contract"
        );
    }

    /// Trace is collected across all transactions in a sequence.
    #[test]
    fn execute_with_trace_collects_calls() {
        let (mut chain, target) = deploy_and_setup_warp();
        chain.config.trace = true;

        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            WarpTarget::actionWarpCall::new(()).abi_encode(),
        ))];

        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert_eq!(execution.results.len(), 1);
        assert!(execution.results[0].success);

        let trace = execution.trace.expect("trace must be present");
        assert!(
            !trace.roots.is_empty(),
            "trace should have at least one root"
        );
    }

    /// A cloned chain should produce independent execution results.
    #[test]
    fn execute_on_cloned_chain_is_isolated() {
        let (mut chain, target) = deploy_and_setup_warp();

        // Mutate original chain.
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            WarpTarget::actionWarpCall::new(()).abi_encode(),
        ))];
        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert!(execution.results[0].success);

        // Clone and run a view call on the clone.
        let mut cloned = chain.clone();
        let view_txs = vec![Transaction::new(target).calldata(Bytes::from(
            WarpTarget::getBlockTimestampCall::new(()).abi_encode(),
        ))];
        let view_input = ExecInput::new(view_txs);
        let view_execution = cloned.exec(view_input).unwrap();
        assert!(view_execution.results[0].success);
        let ts = WarpTarget::getBlockTimestampCall::abi_decode_returns(
            &view_execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(ts, EXPECTED_TIMESTAMP);
    }

    /// Execute against a basic target to verify coverage works with initcode.
    #[test]
    fn execute_coverage_on_basic_target() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/basic-target", "src/NamedMismatch.sol")
                .unwrap();
        let mut chain = Chain::new(Config::default()).unwrap();
        chain.config.coverage = true;
        let deployment = chain.deploy(DeployInput::new(artifact.initcode)).unwrap();
        assert!(deployment.result.success);
        let target = deployment.address.unwrap();

        // `set(uint256)` selector = keccak256("set(uint256)")[:4]
        let set_selector: [u8; 4] = [0x60, 0xfe, 0x47, 0xb1];
        let txs = vec![
            Transaction::new(target)
                .calldata(Bytes::from([set_selector.as_slice(), &[0u8; 32]].concat())),
        ];

        let input = ExecInput::new(txs);
        let execution = chain.exec(input).unwrap();
        assert!(execution.results[0].success);

        let coverage = execution.coverage.expect("coverage must be present");
        assert!(
            !coverage.contracts.is_empty(),
            "coverage should contain at least one contract"
        );
    }
}
