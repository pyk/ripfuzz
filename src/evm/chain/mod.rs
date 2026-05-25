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

use revm::handler::FrameResult;
use revm::interpreter::{CallInputs, CallOutcome, CreateInputs, CreateOutcome, FrameInput};

use crate::chain::inspectors::coverage::CoverageInspector;
use crate::evm::cheatcode::Inspector as CheatcodeInspector;
use crate::evm::database::Database;
use crate::evm::result::TransactionResult;
use crate::evm::trace::{Inspector as TraceInspector, Trace};

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
    pub result: TransactionResult,
    pub trace: Trace,
}

/// Result of a setup call, including the trace.
#[derive(Debug, Clone)]
pub struct SetupOutput {
    pub result: TransactionResult,
    pub trace: Trace,
}

/// Configuration for a setup call.
#[derive(Debug, Clone)]
pub struct SetupInput {
    pub caller: Address,
    pub target: Address,
    pub data: Bytes,
    pub value: U256,
    pub gas_limit: u64,
}

impl SetupInput {
    /// Create [`SetupInput`] for the given target and calldata.
    ///
    /// Caller defaults to [`DEFAULT_DEPLOYER`]; override with [`Self::caller`].
    pub fn new(target: Address, data: Bytes) -> Self {
        Self {
            caller: DEFAULT_DEPLOYER,
            target,
            data,
            value: U256::ZERO,
            gas_limit: u64::MAX,
        }
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
    pub data: Bytes,
    pub value: U256,
    pub gas_limit: u64,
}

impl Transaction {
    /// Create a [`Transaction`] for the given target and calldata.
    ///
    /// Caller defaults to [`DEFAULT_DEPLOYER`]; override with [`Self::caller`].
    pub fn new(target: Address, data: Bytes) -> Self {
        Self {
            caller: DEFAULT_DEPLOYER,
            target,
            data,
            value: U256::ZERO,
            gas_limit: u64::MAX,
        }
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
    pub trace: bool,
    pub cheatcode: bool,
    pub coverage: bool,
}

impl Default for ExecInput {
    fn default() -> Self {
        Self {
            trace: false,
            cheatcode: true,
            coverage: false,
        }
    }
}

/// Result of executing a sequence of transactions.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub results: Vec<TransactionResult>,
    pub trace: Option<Trace>,
    pub coverage: Option<crate::coverage::LocalCoverage>,
}

/// Composite inspector that optionally collects traces, cheatcode state,
/// and coverage across a transaction sequence.
#[derive(Debug)]
struct ExecInspector {
    trace: Option<TraceInspector>,
    cheatcode: Option<CheatcodeInspector>,
    coverage: Option<CoverageInspector>,
}

impl ExecInspector {
    fn new(opts: &ExecInput) -> Self {
        Self {
            trace: if opts.trace {
                Some(TraceInspector::new())
            } else {
                None
            },
            cheatcode: if opts.cheatcode {
                Some(CheatcodeInspector::default())
            } else {
                None
            },
            coverage: if opts.coverage {
                Some(CoverageInspector::new())
            } else {
                None
            },
        }
    }
}

impl Inspector<Context<BlockEnv, TxEnv, CfgEnv, Database, revm::Journal<Database>>>
    for ExecInspector
{
    fn initialize_interp(
        &mut self,
        interp: &mut revm::interpreter::Interpreter<revm::interpreter::interpreter::EthInterpreter>,
        context: &mut Context<BlockEnv, TxEnv, CfgEnv, Database, revm::Journal<Database>>,
    ) {
        if let Some(ref mut t) = self.trace {
            t.initialize_interp(interp, context);
        }
        if let Some(ref mut c) = self.cheatcode {
            c.initialize_interp(interp, context);
        }
        if let Some(ref mut cov) = self.coverage {
            cov.initialize_interp(interp, context);
        }
    }

    fn step(
        &mut self,
        interp: &mut revm::interpreter::Interpreter<revm::interpreter::interpreter::EthInterpreter>,
        context: &mut Context<BlockEnv, TxEnv, CfgEnv, Database, revm::Journal<Database>>,
    ) {
        if let Some(ref mut t) = self.trace {
            t.step(interp, context);
        }
        if let Some(ref mut c) = self.cheatcode {
            c.step(interp, context);
        }
        if let Some(ref mut cov) = self.coverage {
            cov.step(interp, context);
        }
    }

    fn frame_start(
        &mut self,
        context: &mut Context<BlockEnv, TxEnv, CfgEnv, Database, revm::Journal<Database>>,
        frame_input: &mut FrameInput,
    ) -> Option<FrameResult> {
        let mut result = None;
        if let Some(ref mut c) = self.cheatcode {
            result = c.frame_start(context, frame_input);
        }
        if let Some(ref mut t) = self.trace
            && result.is_none()
        {
            result = t.frame_start(context, frame_input);
        }
        if let Some(ref mut cov) = self.coverage
            && result.is_none()
        {
            result = cov.frame_start(context, frame_input);
        }
        result
    }

    fn call(
        &mut self,
        context: &mut Context<BlockEnv, TxEnv, CfgEnv, Database, revm::Journal<Database>>,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        let mut result = None;
        if let Some(ref mut c) = self.cheatcode {
            result = c.call(context, inputs);
        }
        if let Some(ref mut t) = self.trace
            && result.is_none()
        {
            result = t.call(context, inputs);
        }
        if let Some(ref mut cov) = self.coverage
            && result.is_none()
        {
            result = cov.call(context, inputs);
        }
        result
    }

    fn call_end(
        &mut self,
        context: &mut Context<BlockEnv, TxEnv, CfgEnv, Database, revm::Journal<Database>>,
        inputs: &CallInputs,
        outcome: &mut CallOutcome,
    ) {
        if let Some(ref mut c) = self.cheatcode {
            c.call_end(context, inputs, outcome);
        }
        if let Some(ref mut t) = self.trace {
            t.call_end(context, inputs, outcome);
        }
        if let Some(ref mut cov) = self.coverage {
            cov.call_end(context, inputs, outcome);
        }
    }

    fn create(
        &mut self,
        context: &mut Context<BlockEnv, TxEnv, CfgEnv, Database, revm::Journal<Database>>,
        inputs: &mut CreateInputs,
    ) -> Option<CreateOutcome> {
        let mut result = None;
        if let Some(ref mut c) = self.cheatcode {
            result = c.create(context, inputs);
        }
        if let Some(ref mut t) = self.trace
            && result.is_none()
        {
            result = t.create(context, inputs);
        }
        if let Some(ref mut cov) = self.coverage
            && result.is_none()
        {
            result = cov.create(context, inputs);
        }
        result
    }

    fn create_end(
        &mut self,
        context: &mut Context<BlockEnv, TxEnv, CfgEnv, Database, revm::Journal<Database>>,
        inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        if let Some(ref mut c) = self.cheatcode {
            c.create_end(context, inputs, outcome);
        }
        if let Some(ref mut t) = self.trace {
            t.create_end(context, inputs, outcome);
        }
        if let Some(ref mut cov) = self.coverage {
            cov.create_end(context, inputs, outcome);
        }
    }
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

    /// Deploy a contract and return the full [`DeployOutput`] result.
    ///
    /// A [`CheatcodeInspector`] is included so that target contracts can call
    /// raptor cheatcodes (e.g. `vm.warp`) during constructor execution.
    pub fn deploy(&mut self, opts: DeployInput) -> Result<DeployOutput> {
        let inspector = (TraceInspector::new(), CheatcodeInspector::default());
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

    /// Execute a setup CALL against the given target and return the full result with trace.
    pub fn setup(&mut self, opts: SetupInput) -> Result<SetupOutput> {
        let inspector = (TraceInspector::new(), CheatcodeInspector::default());
        let tx = TxEnv {
            caller: opts.caller,
            kind: TxKind::Call(opts.target),
            data: opts.data,
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
    pub fn exec(&mut self, transactions: Vec<Transaction>, opts: ExecInput) -> Result<ExecOutput> {
        let mut inspector = ExecInspector::new(&opts);
        let mut results = Vec::with_capacity(transactions.len());

        for tx in transactions {
            let tx_env = TxEnv {
                caller: tx.caller,
                kind: TxKind::Call(tx.target),
                data: tx.data,
                gas_limit: tx.gas_limit,
                value: tx.value,
                ..Default::default()
            };
            let (result, insp) = self.inspect(tx_env, inspector)?;
            inspector = insp;
            results.push(result);
        }

        Ok(ExecOutput {
            results,
            trace: inspector.trace.map(|t| t.into_trace()),
            coverage: inspector.coverage.map(|c| c.into_coverage()),
        })
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
        self.block_env = evm.ctx.block;
        self.cfg_env = evm.ctx.cfg;
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
        self.block_env = evm.ctx.block;
        self.cfg_env = evm.ctx.cfg;
        Ok((TransactionResult::from(result), evm.inspector))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::contract;
    use crate::evm::chain::{Chain, DeployInput, ExecInput, SetupInput, Transaction};
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
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let setup_data = Bytes::from(WarpTarget::setupCall::new(()).abi_encode());
        let setup_opts = SetupInput::new(target, setup_data);
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
            Transaction::new(
                target,
                Bytes::from(WarpTarget::actionWarpCall::new(()).abi_encode()),
            ),
            Transaction::new(
                target,
                Bytes::from(WarpTarget::invariant_warpCall::new(()).abi_encode()),
            ),
        ];

        let execution = chain.exec(txs, ExecInput::default()).unwrap();
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

        let txs = vec![
            Transaction::new(
                target,
                Bytes::from(WarpTarget::actionWarpCall::new(()).abi_encode()),
            ),
            Transaction::new(
                target,
                Bytes::from(WarpTarget::getBlockTimestampCall::new(()).abi_encode()),
            ),
        ];

        let opts = ExecInput {
            coverage: true,
            ..ExecInput::default()
        };
        let execution = chain.exec(txs, opts).unwrap();
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

        let txs = vec![Transaction::new(
            target,
            Bytes::from(WarpTarget::actionWarpCall::new(()).abi_encode()),
        )];

        let opts = ExecInput {
            trace: true,
            ..ExecInput::default()
        };
        let execution = chain.exec(txs, opts).unwrap();
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
        let txs = vec![Transaction::new(
            target,
            Bytes::from(WarpTarget::actionWarpCall::new(()).abi_encode()),
        )];
        let execution = chain.exec(txs, ExecInput::default()).unwrap();
        assert!(execution.results[0].success);

        // Clone and run a view call on the clone.
        let mut cloned = chain.clone();
        let view_txs = vec![Transaction::new(
            target,
            Bytes::from(WarpTarget::getBlockTimestampCall::new(()).abi_encode()),
        )];
        let view_execution = cloned.exec(view_txs, ExecInput::default()).unwrap();
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
        let mut chain = Chain::empty();
        let deployment = chain.deploy(DeployInput::new(artifact.initcode)).unwrap();
        assert!(deployment.result.success);
        let target = deployment.address.unwrap();

        // `set(uint256)` selector = keccak256("set(uint256)")[:4]
        let set_selector: [u8; 4] = [0x60, 0xfe, 0x47, 0xb1];
        let txs = vec![Transaction::new(
            target,
            Bytes::from([set_selector.as_slice(), &[0u8; 32]].concat()),
        )];

        let opts = ExecInput {
            coverage: true,
            ..ExecInput::default()
        };
        let execution = chain.exec(txs, opts).unwrap();
        assert!(execution.results[0].success);

        let coverage = execution.coverage.expect("coverage must be present");
        assert!(
            !coverage.contracts.is_empty(),
            "coverage should contain at least one contract"
        );
    }
}
