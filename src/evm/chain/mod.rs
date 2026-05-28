//! EVM chain state and executor.

use std::collections::HashMap;

use alloy_primitives::{Address, U256, address, keccak256};
use alloy_sol_types::SolCall;
use anyhow::{Context as _, Result, ensure};
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
    pub initcode: String,
    pub libraries: Vec<DeployLibraryInput>,
    pub gas_limit: u64,
}

/// Configuration for a linked library deployment.
#[derive(Debug, Clone)]
pub struct DeployLibraryInput {
    pub id: String,
    pub initcode: String,
    pub libraries: Vec<DeployLibraryInput>,
}

impl DeployLibraryInput {
    /// Create [`DeployLibraryInput`] with the given identifier and initcode.
    pub fn new(id: impl Into<String>, initcode: &str) -> Self {
        Self {
            id: id.into(),
            initcode: initcode.into(),
            libraries: Vec::new(),
        }
    }

    /// Add a nested library dependency.
    pub fn add_library(mut self, library: DeployLibraryInput) -> Self {
        self.libraries.push(library);
        self
    }
}

impl DeployInput {
    /// Create [`DeployInput`] with the given initcode.
    ///
    /// Caller defaults to [`DEFAULT_DEPLOYER`]; override with [`Self::caller`].
    pub fn new(initcode: &str) -> Self {
        Self {
            caller: DEFAULT_DEPLOYER,
            value: U256::ZERO,
            initcode: initcode.into(),
            libraries: Vec::new(),
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

    /// Add a linked library to deploy before the target contract.
    pub fn add_library(mut self, library: DeployLibraryInput) -> Self {
        self.libraries.push(library);
        self
    }

    /// Set the gas limit for the deployment transaction.
    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.gas_limit = gas_limit;
        self
    }
}

/// Result of a deployed library.
#[derive(Debug, Clone)]
pub struct DeployLibraryOutput {
    pub id: String,
    pub address: Address,
}

/// Result of a contract deployment, including the trace.
///
/// `address` is `None` when the constructor reverts or halts, but `result`
/// and `trace` are still populated so the caller can inspect the failure.
#[derive(Debug, Clone)]
pub struct DeployOutput {
    pub address: Option<Address>,
    pub libraries: Vec<DeployLibraryOutput>,
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

/// Result of executing a sequence of transactions.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub results: Vec<result::TransactionResult>,
    pub trace: Option<trace::Trace>,
    pub coverage: Option<coverage::exec::ExecutionCoverage>,
    pub panic_transactions: Vec<Transaction>,
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
    /// Snapshotted cheatcode inspector state after deploy and setup.
    ///
    /// Required so that `vm.label`, `vm.prank`, `vm.warp`, and other
    /// cheatcodes that mutate inspector state during setup are visible
    /// to actions and invariants during `chain.exec`.
    pub cheatcode_state: cheatcode::ExecutionState,
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
    ///
    /// If `opts.libraries` is non-empty, the linked libraries are deployed first
    /// (recursively, in dependency order), their addresses are collected, and the
    /// target contract initcode is linked before deployment.
    pub fn deploy(&mut self, opts: DeployInput) -> Result<DeployOutput> {
        let library_addrs = self.deploy_libraries(opts.libraries, opts.caller)?;

        let initcode = if library_addrs.is_empty() {
            opts.initcode
        } else {
            self.link_libraries(&opts.initcode, &library_addrs)
        };

        let mut output = self.deploy_raw(
            opts.caller,
            opts.value,
            initcode.parse().unwrap_or_default(),
            opts.gas_limit,
        )?;
        output.libraries = library_addrs
            .into_iter()
            .map(|(id, address)| DeployLibraryOutput { id, address })
            .collect();
        Ok(output)
    }

    /// Deploy a list of libraries and return a map of their identifiers to
    /// deployed addresses.
    ///
    /// Libraries are deployed recursively (nested dependencies first) and
    /// shared dependencies are deduplicated.
    pub fn deploy_libraries(
        &mut self,
        libraries: Vec<DeployLibraryInput>,
        deployer: Address,
    ) -> Result<HashMap<String, Address>> {
        let mut library_addrs = HashMap::new();
        for lib in libraries {
            self.deploy_library(&lib, deployer, &mut library_addrs)?;
        }
        Ok(library_addrs)
    }

    /// Deploy a single library (including its nested dependencies) and return
    /// the deployment result.
    ///
    /// `library_addrs` is used to deduplicate shared dependencies and to link
    /// the library initcode with already-deployed libraries.
    pub fn deploy_library(
        &mut self,
        lib: &DeployLibraryInput,
        deployer: Address,
        library_addrs: &mut HashMap<String, Address>,
    ) -> Result<DeployLibraryOutput> {
        // Deploy nested dependencies first.
        for nested in &lib.libraries {
            self.deploy_library(nested, deployer, library_addrs)?;
        }

        let identifier = lib.id.clone();

        // Skip if this library was already deployed (e.g. shared dependency).
        if let Some(&addr) = library_addrs.get(&identifier) {
            return Ok(DeployLibraryOutput {
                id: identifier,
                address: addr,
            });
        }

        // Link the library initcode with already-deployed libraries.
        let initcode = self.link_libraries(&lib.initcode, library_addrs);

        // Deploy the library and return its output.
        let deployment = self.deploy_raw(
            deployer,
            U256::ZERO,
            initcode.parse().unwrap_or_default(),
            u64::MAX,
        )?;
        ensure!(
            deployment.result.success,
            "library deployment failed: {} (output: {:?})",
            identifier,
            deployment.result.output
        );
        let address = deployment
            .address
            .with_context(|| format!("library deployment missing address: {}", identifier))?;

        library_addrs.insert(identifier.clone(), address);
        Ok(DeployLibraryOutput {
            id: identifier,
            address,
        })
    }

    /// Compute the Solidity placeholder string for a library identifier.
    ///
    /// The placeholder format is `__$<keccak256(identifier)[:34]>$__`.
    pub fn get_library_placeholder(&self, identifier: &str) -> String {
        let hash = keccak256(identifier.as_bytes());
        let hex = alloy_primitives::hex::encode(hash);
        format!("__${}$__", &hex[..34])
    }

    /// Replace library placeholders in initcode with deployed addresses.
    pub fn link_libraries(&self, initcode: &str, libraries: &HashMap<String, Address>) -> String {
        let mut hex = initcode.to_owned();
        for (identifier, address) in libraries {
            let placeholder = self.get_library_placeholder(identifier);
            let address_hex = hex::encode(address);
            hex = hex.replace(&placeholder, &address_hex);
        }
        hex
    }

    /// Execute a raw CREATE transaction without library handling.
    fn deploy_raw(
        &mut self,
        caller: Address,
        value: U256,
        initcode: Bytes,
        gas_limit: u64,
    ) -> Result<DeployOutput> {
        let inspector = (
            trace::Inspector::new(),
            cheatcode::Inspector::from_state(self.cheatcode_state.clone()),
        );
        let tx = TxEnv {
            caller,
            kind: TxKind::Create,
            data: initcode,
            gas_limit,
            value,
            ..Default::default()
        };
        let (result, (trace_inspector, cheatcode_inspector)) = self.inspect(tx, inspector)?;
        self.cheatcode_state = cheatcode_inspector.state;
        let address = result.created_address;
        let trace = trace_inspector.into_trace();
        Ok(DeployOutput {
            address,
            libraries: Vec::new(),
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
            cheatcode::Inspector::from_state(self.cheatcode_state.clone()),
        );
        let tx = TxEnv {
            caller: opts.caller,
            kind: TxKind::Call(opts.target),
            data: opts.calldata,
            gas_limit: opts.gas_limit,
            value: opts.value,
            ..Default::default()
        };
        let (result, (trace_inspector, cheatcode_inspector)) = self.inspect(tx, inspector)?;
        self.cheatcode_state = cheatcode_inspector.state;
        let trace = trace_inspector.into_trace();
        Ok(SetupOutput { result, trace })
    }

    /// Execute a sequence of transactions and commit state changes.
    ///
    /// The same inspector is reused across all transactions, so cheatcode
    /// effects (e.g. `vm.warp`) and coverage collection persist from one
    /// transaction to the next.
    pub fn exec(&mut self, transactions: &[Transaction]) -> Result<ExecOutput> {
        let inspector = (
            cheatcode::Inspector::from_state(self.cheatcode_state.clone()),
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
        let mut results = Vec::with_capacity(transactions.len());
        let mut panic_transactions = Vec::new();

        let db = self.database.take().context("database unavailable")?;
        let mut ctx = Context::mainnet().with_db(db);
        ctx.block = self.block_env.clone();
        ctx.cfg = self.cfg_env.clone();
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        for tx in transactions {
            let tx_env = TxEnv {
                caller: tx.caller,
                kind: TxKind::Call(tx.target),
                // checkrs: allow(clone_in_loops)
                data: tx.calldata.clone(),
                gas_limit: tx.gas_limit,
                value: tx.value,
                ..Default::default()
            };
            let result = evm
                .inspect_tx_commit(tx_env)
                .context("revm transaction failed")?;
            let result = result::TransactionResult::from(result);
            if let Some(ref output) = result.output
                && result::is_assert_failure(output)
            {
                // checkrs: allow(clone_in_loops)
                panic_transactions.push(tx.clone());
            }
            results.push(result);
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
            panic_transactions,
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

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, Config, DeployInput, SetupInput, Transaction};
    use crate::foundry;

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
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup_warp() -> (Chain, Address) {
        let contract = load_warp_fixture();
        let mut chain = Chain::new(Config::default()).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
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

        let execution = chain.exec(&txs).unwrap();
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

        let execution = chain.exec(&txs).unwrap();
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

        let execution = chain.exec(&txs).unwrap();
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

        let execution = chain.exec(&txs).unwrap();
        assert!(execution.results[0].success);

        // Clone and run a view call on the clone.
        let mut cloned = chain.clone();
        let view_txs = vec![Transaction::new(target).calldata(Bytes::from(
            WarpTarget::getBlockTimestampCall::new(()).abi_encode(),
        ))];
        let view_execution = cloned.exec(&view_txs).unwrap();
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
        let project = foundry::Project::new("fixtures/basic-target");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id =
            foundry::ArtifactId::try_from("src/NamedMismatch.sol:DifferentName").unwrap();
        let contract = Contract::try_get(&artifacts, &artifact_id).unwrap();
        let mut chain = Chain::new(Config::default()).unwrap();
        chain.config.coverage = true;
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success);
        let target = deployment.address.unwrap();

        // `set(uint256)` selector = keccak256("set(uint256)")[:4]
        let set_selector: [u8; 4] = [0x60, 0xfe, 0x47, 0xb1];
        let txs = vec![
            Transaction::new(target)
                .calldata(Bytes::from([set_selector.as_slice(), &[0u8; 32]].concat())),
        ];

        let execution = chain.exec(&txs).unwrap();
        assert!(execution.results[0].success);

        let coverage = execution.coverage.expect("coverage must be present");
        assert!(
            !coverage.contracts.is_empty(),
            "coverage should contain at least one contract"
        );
    }
}
