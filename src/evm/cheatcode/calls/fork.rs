//! `fork` cheatcode - create or select a remote chain fork.

use std::sync::Arc;

use alloy_primitives::Address;
use revm::{
    DatabaseCommit, Journal, JournalEntry,
    context::{BlockEnv, ContextSetters},
    context_interface::block::BlobExcessGasAndPrice,
    context_interface::{ContextTr, JournalTr},
    primitives::U256,
    state::EvmState,
};

use crate::evm::chain::DEFAULT_DEPLOYER;
use crate::evm::cheatcode::VM_ADDRESS;
use crate::evm::cheatcode::inspector::CfgMut;
use crate::evm::cheatcode::{outcome, state::ExecutionState};
use crate::evm::database::{Database, DatabaseError, ForkEnv};
use crate::evm::forkdb::{ForkDBConfig, SharedLocalAddressRegistry, Transport};

/// Runtime fork options from the optional Solidity `ForkConfig` argument.
#[derive(Debug, Clone, Copy, Default)]
pub struct ForkOptions {
    pub retries: Option<u32>,
    pub backoff_ms: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub rate_limit: Option<u64>,
}

/// `vm.fork(url, blockNumber)` - create or select a fork with campaign defaults.
pub fn fork<CTX>(
    ctx: &mut CTX,
    state: &mut ExecutionState,
    url: &str,
    block_number: U256,
) -> Option<revm::interpreter::CallOutcome>
where
    CTX: ContextTr + ContextSetters<Block = BlockEnv> + CfgMut,
    CTX::Db: AsForkDatabase,
    CTX::Journal: CommitRemoteBeforeForkSwitch,
{
    fork_with_options(ctx, state, url, block_number, ForkOptions::default())
}

/// `vm.fork(url, blockNumber, config)` - create or select a fork with overrides.
pub fn fork_with_options<CTX>(
    ctx: &mut CTX,
    state: &mut ExecutionState,
    url: &str,
    block_number: U256,
    options: ForkOptions,
) -> Option<revm::interpreter::CallOutcome>
where
    CTX: ContextTr + ContextSetters<Block = BlockEnv> + CfgMut,
    CTX::Db: AsForkDatabase,
    CTX::Journal: CommitRemoteBeforeForkSwitch,
{
    if url.is_empty() {
        return Some(outcome::revert("fork: empty RPC URL"));
    }

    let block = match u64::try_from(block_number) {
        Ok(n) => n,
        Err(_) => {
            return Some(outcome::revert(&format!(
                "fork: block number {block_number} does not fit in u64"
            )));
        }
    };

    let mut config = state.fork_defaults.clone();
    config.url = url.to_owned();
    config.block_number = block;
    if let Some(retries) = options.retries {
        config.retries = retries;
    }
    if let Some(backoff_ms) = options.backoff_ms {
        config.backoff_ms = backoff_ms;
    }
    if let Some(timeout_ms) = options.timeout_ms {
        config.timeout_ms = timeout_ms;
    }
    match options.rate_limit {
        Some(0) => config.rate_limit = None,
        Some(rate_limit) => config.rate_limit = Some(rate_limit),
        None => {}
    }

    let transport = state.transport.clone();
    let registry = state.local_registry.clone();

    // Before switching overlays mid-transaction, commit remote journaled state
    // to the currently active fork and drop those accounts from the journal.
    // Otherwise tx-end commit lands only on the post-switch fork and earlier
    // `rvm.store` / `rvm.deal` mutations are lost (or leak to the wrong chain).
    // Local accounts stay journaled so harness storage remains shared.
    let needs_remote_commit = !ctx.journal().db().is_active_fork(url, block);
    if needs_remote_commit {
        let registry_for_commit = registry.clone();
        ctx.journal_mut()
            .commit_remote_before_fork_switch(&move |addr| {
                addr == DEFAULT_DEPLOYER || addr == VM_ADDRESS || registry_for_commit.is_local(addr)
            });
    }

    let env = match ctx
        .journal_mut()
        .db_mut()
        .fork(url, block, config, transport, registry)
    {
        Ok(env) => env,
        Err(e) => return Some(outcome::revert(&format!("fork failed: {e}"))),
    };

    // Apply forked block and chain environment to the active EVM context.
    let mut block_env = ctx.block().clone();
    block_env.number = U256::from(env.block_number);
    block_env.timestamp = env.timestamp;
    block_env.beneficiary = env.beneficiary;
    block_env.difficulty = env.difficulty;
    block_env.prevrandao = env.prevrandao;
    block_env.gas_limit = u64::MAX;
    block_env.basefee = 0;
    // Post-Cancun specs require blob excess gas; default to 0 when the remote
    // header omits it (common for pre-Cancun chains and simple mocks).
    let excess = env.excess_blob_gas.unwrap_or(0);
    block_env.blob_excess_gas_and_price =
        Some(BlobExcessGasAndPrice::new_with_spec(excess, env.spec_id));
    ctx.set_block(block_env);

    ctx.set_chain_id(env.chain_id);
    // Match the forked block's hardfork so opcodes and gas params align with
    // the remote chain at that height (same as the former Chain::fork path).
    ctx.set_spec_and_mainnet_gas_params(env.spec_id);
    // Keep cheatcode block scratchpad in sync so later txs inherit values.
    state.block.chain_id = Some(U256::from(env.chain_id));
    state.block.number = Some(U256::from(env.block_number));
    state.block.timestamp = Some(env.timestamp);
    state.block.beneficiary = Some(env.beneficiary);
    state.block.prevrandao = env.prevrandao;

    Some(outcome::success())
}

/// Trait implemented by [`Database`] so the fork cheatcode can switch backends.
pub trait AsForkDatabase {
    fn is_active_fork(&self, url: &str, block_number: u64) -> bool;

    fn fork(
        &mut self,
        url: &str,
        block_number: u64,
        config: ForkDBConfig,
        transport: Option<Arc<dyn Transport>>,
        local_registry: SharedLocalAddressRegistry,
    ) -> Result<ForkEnv, DatabaseError>;
}

impl AsForkDatabase for Database {
    fn is_active_fork(&self, url: &str, block_number: u64) -> bool {
        Database::is_active_fork(self, url, block_number)
    }

    fn fork(
        &mut self,
        url: &str,
        block_number: u64,
        config: ForkDBConfig,
        transport: Option<Arc<dyn Transport>>,
        local_registry: SharedLocalAddressRegistry,
    ) -> Result<ForkEnv, DatabaseError> {
        Database::fork(self, url, block_number, config, transport, local_registry)
    }
}

/// Commit remote journaled mutations onto the active fork overlay before a
/// mid-transaction fork switch, and drop those accounts from the in-memory
/// journal so they are reloaded from the selected fork.
///
/// Local (persistent) accounts stay in the journal so harness storage continues
/// to be shared across forks.
pub trait CommitRemoteBeforeForkSwitch {
    fn commit_remote_before_fork_switch(&mut self, is_persistent: &dyn Fn(Address) -> bool);
}

impl CommitRemoteBeforeForkSwitch for Journal<Database> {
    fn commit_remote_before_fork_switch(&mut self, is_persistent: &dyn Fn(Address) -> bool) {
        let remote_addrs: Vec<Address> = self
            .inner
            .state
            .keys()
            .copied()
            .filter(|addr| !is_persistent(*addr))
            .collect();

        if remote_addrs.is_empty() {
            return;
        }

        let mut remote_changes = EvmState::default();
        for addr in &remote_addrs {
            if let Some(account) = self.inner.state.remove(addr) {
                remote_changes.insert(*addr, account);
            }
        }

        if !remote_changes.is_empty() {
            self.database.commit(remote_changes);
        }

        // Journal entry reverts use `.unwrap()` on state lookups. Drop entries
        // that touch remote accounts so a later call revert cannot panic after
        // those accounts were removed from the journaled state.
        self.inner
            .journal
            .retain(|entry| journal_entry_is_persistent(entry, is_persistent));
    }
}

fn journal_entry_is_persistent(
    entry: &JournalEntry,
    is_persistent: &dyn Fn(Address) -> bool,
) -> bool {
    match entry {
        JournalEntry::AccountWarmed { address }
        | JournalEntry::AccountTouched { address }
        | JournalEntry::BalanceChange { address, .. }
        | JournalEntry::NonceChange { address, .. }
        | JournalEntry::NonceBump { address }
        | JournalEntry::AccountCreated { address, .. }
        | JournalEntry::StorageChanged { address, .. }
        | JournalEntry::StorageWarmed { address, .. }
        | JournalEntry::TransientStorageChange { address, .. }
        | JournalEntry::CodeChange { address } => is_persistent(*address),
        JournalEntry::AccountDestroyed {
            address, target, ..
        } => is_persistent(*address) && is_persistent(*target),
        JournalEntry::BalanceTransfer { from, to, .. } => {
            is_persistent(*from) && is_persistent(*to)
        }
    }
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;
    use serde_json::json;

    use crate::compilers::solc::{Solc, SolcOutput};
    use crate::evm::SharedCoverage;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::forkdb::{ForkDBConfig, MockTransport};
    use crate::harness::HarnessId;

    fn compile_fixture(root: &str, target: &str) -> SolcOutput {
        let id = HarnessId::try_from(target).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        Solc::new()
            .with_version("0.8.36")
            .with_root(root)
            .with_target(&id.path)
            .with_name(&id.name)
            .with_out(tmp.path().join("out"))
            .compile()
            .unwrap()
    }

    alloy_sol_types::sol! {
        interface ForkHarness {
            function setup() external;
            function actionFork(string calldata url, uint256 blockNumber) external;
            function actionForkAndReadBridge(string calldata url, uint256 blockNumber) external;
            function actionForkStoreBridge(
                string calldata url,
                uint256 blockNumber,
                bytes32 value
            ) external;
            function actionForkDealBridge(
                string calldata url,
                uint256 blockNumber,
                uint256 value
            ) external;
            function actionForkStoreThenSwitch(
                string calldata urlA,
                uint256 blockA,
                bytes32 value,
                string calldata urlB,
                uint256 blockB
            ) external;
            function actionForkDealThenSwitch(
                string calldata urlA,
                uint256 blockA,
                uint256 value,
                string calldata urlB,
                uint256 blockB
            ) external;
            function actionReadBridge() external;
            function getBlockNumber() external view returns (uint256);
            function getChainId() external view returns (uint256);
            function getTimestamp() external view returns (uint256);
            function getLastSlot0() external view returns (bytes32);
            function getLastBalance() external view returns (uint256);
        }
    }

    /// Same address on both chains (PolyBridger-style).
    const BRIDGE: &str = "0x1111111111111111111111111111111111111111";
    const URL_ETH: &str = "mock://ethereum";
    const URL_POLYGON: &str = "mock://polygon";
    const BLOCK_ETH: u64 = 21_000_000;
    const BLOCK_POLYGON: u64 = 50_000_000;

    fn mock_fork_setup(
        transport: &MockTransport,
        url: &str,
        block_number: u64,
        chain_id_hex: &str,
        timestamp_hex: &str,
    ) {
        let chain_id_payload = json!([
            {"jsonrpc":"2.0","id":0,"method":"eth_chainId","params":[]}
        ]);
        let block_payload = json!([
            {
                "jsonrpc":"2.0",
                "id":0,
                "method":"eth_getBlockByNumber",
                "params":[json!(format!("0x{block_number:x}")), json!(false)]
            }
        ]);
        transport.mock_response(
            url,
            &chain_id_payload,
            json!([{"jsonrpc":"2.0","id":0,"result":chain_id_hex}]),
        );
        transport.mock_response(
            url,
            &block_payload,
            json!([{
                "jsonrpc":"2.0",
                "id":0,
                "result":{
                    "number": format!("0x{block_number:x}"),
                    "timestamp": timestamp_hex,
                    "miner":"0x0000000000000000000000000000000000000000",
                    "gasLimit":"0xffffffffffffffff",
                    "baseFeePerGas":"0x0",
                    "difficulty":"0x0",
                    "mixHash":"0x0000000000000000000000000000000000000000000000000000000000000000",
                    "hash":"0x0000000000000000000000000000000000000000000000000000000000000000"
                }
            }]),
        );
    }

    fn load_initcode(id: &str) -> String {
        compile_fixture("fixtures/harness-contract-with-cheatcodes", id)
            .initcode()
            .unwrap()
            .to_owned()
    }

    fn deploy_with_transport(transport: MockTransport) -> (Chain, Address) {
        let initcode = load_initcode("ForkHarness.sol:ForkHarness");
        let config = ChainConfig::default()
            .with_transport(Arc::new(transport))
            .with_fork_defaults(ForkDBConfig::new(""));
        let mut chain = Chain::new(config).unwrap();
        let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();
        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success, "setup must succeed");
        (chain, target)
    }

    /// `vm.fork` from an action must pin block number and chain id from the RPC.
    #[test]
    fn fork_in_action_sets_block_env() {
        let transport = MockTransport::default();
        let url = "mock://fork-a";
        // Base Ecotone-era timestamp so SpecId is Cancun (PUSH0-safe for solc prague).
        mock_fork_setup(&transport, url, 42, "0x2105", "0x65f5e100");

        let (mut chain, target) = deploy_with_transport(transport);
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((url.to_string(), U256::from(42))).abi_encode(),
        ))];
        let execution = chain.exec(&txs).unwrap();
        assert!(execution.results[0].success, "actionFork must succeed");

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::getBlockNumberCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::getChainIdCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::getTimestampCall::new(()).abi_encode(),
            )),
        ];
        let execution = chain.exec(&txs).unwrap();
        assert!(execution.results.iter().all(|r| r.success));

        let number = ForkHarness::getBlockNumberCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        let chain_id = ForkHarness::getChainIdCall::abi_decode_returns(
            &execution.results[1].output.clone().unwrap(),
        )
        .unwrap();
        let timestamp = ForkHarness::getTimestampCall::abi_decode_returns(
            &execution.results[2].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(number, U256::from(42));
        assert_eq!(chain_id, U256::from(0x2105));
        assert_eq!(timestamp, U256::from(0x65f5e100u64));
    }

    /// Switching forks must update the active chain id and block independently.
    #[test]
    fn multi_fork_switch_updates_env() {
        let transport = MockTransport::default();
        let url_a = "mock://fork-a";
        let url_b = "mock://fork-b";
        // Cancun-era timestamps so SpecId stays PUSH0-safe for solc prague bytecode.
        mock_fork_setup(&transport, url_a, 10, "0x1", "0x65f5e100");
        mock_fork_setup(&transport, url_b, 20, "0x89", "0x65f5e100");

        let (mut chain, target) = deploy_with_transport(transport);

        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((url_a.to_string(), U256::from(10))).abi_encode(),
        ))];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((url_b.to_string(), U256::from(20))).abi_encode(),
        ))];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::getBlockNumberCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::getChainIdCall::new(()).abi_encode(),
            )),
        ];
        let execution = chain.exec(&txs).unwrap();
        let number = ForkHarness::getBlockNumberCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        let chain_id = ForkHarness::getChainIdCall::abi_decode_returns(
            &execution.results[1].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(number, U256::from(20));
        assert_eq!(chain_id, U256::from(0x89));

        // Switch back to A; prior fork overlay must still be selectable.
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((url_a.to_string(), U256::from(10))).abi_encode(),
        ))];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getBlockNumberCall::new(()).abi_encode(),
        ))];
        let execution = chain.exec(&txs).unwrap();
        let number = ForkHarness::getBlockNumberCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(number, U256::from(10));
    }

    /// Fuzzer workers clone the post-setup chain. After `rvm.fork`, a clone must
    /// keep the forked block env, chain id, and remote state cache so handlers
    /// see the same fork without re-fetching chain_id/block.
    #[test]
    fn cloned_chain_preserves_fork_env() {
        let transport = MockTransport::default();
        let url = "mock://fork-clone";
        mock_fork_setup(&transport, url, 42, "0x2105", "0x65f5e100");

        let (mut chain, target) = deploy_with_transport(transport.clone());
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((url.to_string(), U256::from(42))).abi_encode(),
        ))];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        let calls_after_fork = transport.total_calls();
        assert_eq!(calls_after_fork, 2, "fork init is chain_id + block");

        let mut cloned = chain.clone();
        assert_eq!(cloned.cfg_env().chain_id, 0x2105);
        assert_eq!(cloned.block_env().number, U256::from(42));
        assert_eq!(cloned.block_env().timestamp, U256::from(0x65f5e100u64));
        assert_eq!(cloned.block_env().basefee, 0);
        assert_eq!(cloned.block_env().gas_limit, u64::MAX);

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::getBlockNumberCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::getChainIdCall::new(()).abi_encode(),
            )),
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::getTimestampCall::new(()).abi_encode(),
            )),
        ];
        let execution = cloned.exec(&txs).unwrap();
        assert!(
            execution.results.iter().all(|r| r.success),
            "cloned chain must execute view calls against the forked env"
        );
        let number = ForkHarness::getBlockNumberCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        let chain_id = ForkHarness::getChainIdCall::abi_decode_returns(
            &execution.results[1].output.clone().unwrap(),
        )
        .unwrap();
        let timestamp = ForkHarness::getTimestampCall::abi_decode_returns(
            &execution.results[2].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(number, U256::from(42));
        assert_eq!(chain_id, U256::from(0x2105));
        assert_eq!(timestamp, U256::from(0x65f5e100u64));
        assert_eq!(
            transport.total_calls(),
            calls_after_fork,
            "clone must not re-fetch chain_id/block"
        );
    }

    /// Empty URL must revert.
    #[test]
    fn fork_empty_url_reverts() {
        let transport = MockTransport::default();
        let (mut chain, target) = deploy_with_transport(transport);
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionForkCall::new((String::new(), U256::from(1))).abi_encode(),
        ))];
        let execution = chain.exec(&txs).unwrap();
        assert!(
            !execution.results[0].success,
            "empty URL must cause actionFork to revert"
        );
    }

    /// Mock balance/nonce/code + storage slot 0 for a remote account on one fork.
    fn mock_bridge_account(
        transport: &MockTransport,
        url: &str,
        block: u64,
        balance_hex: &str,
        slot0_hex: &str,
    ) {
        let block_hex = format!("0x{block:x}");
        let basic_payload = json!([
            {
                "jsonrpc": "2.0",
                "id": 0,
                "method": "eth_getBalance",
                "params": [BRIDGE, block_hex]
            },
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "eth_getTransactionCount",
                "params": [BRIDGE, block_hex]
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "eth_getCode",
                "params": [BRIDGE, block_hex]
            }
        ]);
        transport.mock_response(
            url,
            &basic_payload,
            json!([
                {"jsonrpc": "2.0", "id": 0, "result": balance_hex},
                {"jsonrpc": "2.0", "id": 1, "result": "0x0"},
                {"jsonrpc": "2.0", "id": 2, "result": "0x"}
            ]),
        );

        let storage_payload = json!([
            {
                "jsonrpc": "2.0",
                "id": 0,
                "method": "eth_getStorageAt",
                "params": [BRIDGE, "0x0", block_hex]
            }
        ]);
        transport.mock_response(
            url,
            &storage_payload,
            json!([{"jsonrpc": "2.0", "id": 0, "result": slot0_hex}]),
        );
    }

    fn setup_eth_polygon_forks(transport: &MockTransport) {
        // Ethereum: chain id 1, slot0 = 1, balance = 100
        // Cancun-era timestamps so SpecId stays PUSH0-safe for solc prague bytecode.
        mock_fork_setup(transport, URL_ETH, BLOCK_ETH, "0x1", "0x65f5e100");
        mock_bridge_account(transport, URL_ETH, BLOCK_ETH, "0x64", "0x1");

        // Polygon: chain id 137, slot0 = 2, balance = 200
        mock_fork_setup(transport, URL_POLYGON, BLOCK_POLYGON, "0x89", "0x65f5e100");
        mock_bridge_account(transport, URL_POLYGON, BLOCK_POLYGON, "0xc8", "0x2");
    }

    fn read_last_slot0(chain: &mut Chain, target: Address) -> alloy_primitives::FixedBytes<32> {
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getLastSlot0Call::new(()).abi_encode(),
        ))];
        let execution = chain.exec(&txs).unwrap();
        assert!(execution.results[0].success, "getLastSlot0 must succeed");
        ForkHarness::getLastSlot0Call::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap()
    }

    fn read_last_balance(chain: &mut Chain, target: Address) -> U256 {
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getLastBalanceCall::new(()).abi_encode(),
        ))];
        let execution = chain.exec(&txs).unwrap();
        assert!(execution.results[0].success, "getLastBalance must succeed");
        ForkHarness::getLastBalanceCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap()
    }

    /// Same address on Ethereum and Polygon must expose different remote storage.
    #[test]
    fn same_address_isolated_storage_across_chains() {
        let transport = MockTransport::default();
        setup_eth_polygon_forks(&transport);
        let (mut chain, target) = deploy_with_transport(transport);

        // Fork Ethereum: bridge slot0 == 1
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkAndReadBridgeCall::new((
                    URL_ETH.to_string(),
                    U256::from(BLOCK_ETH),
                ))
                .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(
            read_last_slot0(&mut chain, target),
            alloy_primitives::FixedBytes::from(U256::from(1).to_be_bytes()),
            "ethereum bridge slot0 must be 1"
        );

        // Fork Polygon: bridge slot0 == 2 (same address, different chain)
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkAndReadBridgeCall::new((
                    URL_POLYGON.to_string(),
                    U256::from(BLOCK_POLYGON),
                ))
                .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(
            read_last_slot0(&mut chain, target),
            alloy_primitives::FixedBytes::from(U256::from(2).to_be_bytes()),
            "polygon bridge slot0 must be 2"
        );
    }

    /// Same address on Ethereum and Polygon must expose different remote balances.
    #[test]
    fn same_address_isolated_balance_across_chains() {
        let transport = MockTransport::default();
        setup_eth_polygon_forks(&transport);
        let (mut chain, target) = deploy_with_transport(transport);

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkAndReadBridgeCall::new((
                    URL_ETH.to_string(),
                    U256::from(BLOCK_ETH),
                ))
                .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(
            read_last_balance(&mut chain, target),
            U256::from(100),
            "ethereum bridge balance must be 100"
        );

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkAndReadBridgeCall::new((
                    URL_POLYGON.to_string(),
                    U256::from(BLOCK_POLYGON),
                ))
                .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(
            read_last_balance(&mut chain, target),
            U256::from(200),
            "polygon bridge balance must be 200"
        );
    }

    /// Mutating remote storage on chain A must not affect chain B; switching back
    /// must keep A's mutation.
    #[test]
    fn store_on_one_fork_does_not_leak_to_other() {
        let transport = MockTransport::default();
        setup_eth_polygon_forks(&transport);
        let (mut chain, target) = deploy_with_transport(transport);

        let mutated = alloy_primitives::FixedBytes::from(U256::from(99).to_be_bytes());

        // Mutate Ethereum bridge slot0 -> 99
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkStoreBridgeCall::new((
                    URL_ETH.to_string(),
                    U256::from(BLOCK_ETH),
                    mutated,
                ))
                .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(read_last_slot0(&mut chain, target), mutated);

        // Polygon still has remote value 2
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkAndReadBridgeCall::new((
                    URL_POLYGON.to_string(),
                    U256::from(BLOCK_POLYGON),
                ))
                .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(
            read_last_slot0(&mut chain, target),
            alloy_primitives::FixedBytes::from(U256::from(2).to_be_bytes()),
            "polygon must be unaffected by ethereum store"
        );

        // Switch back to Ethereum: mutation must still be present in eth overlay
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkCall::new((URL_ETH.to_string(), U256::from(BLOCK_ETH)))
                    .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionReadBridgeCall::new(()).abi_encode(),
        ))];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(
            read_last_slot0(&mut chain, target),
            mutated,
            "ethereum mutation must persist after switching back"
        );
    }

    /// Mutating balance via vm.deal on one fork must not affect the other fork.
    #[test]
    fn deal_on_one_fork_does_not_leak_to_other() {
        let transport = MockTransport::default();
        setup_eth_polygon_forks(&transport);
        let (mut chain, target) = deploy_with_transport(transport);

        // deal Ethereum bridge to 999
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkDealBridgeCall::new((
                    URL_ETH.to_string(),
                    U256::from(BLOCK_ETH),
                    U256::from(999),
                ))
                .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(read_last_balance(&mut chain, target), U256::from(999));

        // Polygon still has remote balance 200
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkAndReadBridgeCall::new((
                    URL_POLYGON.to_string(),
                    U256::from(BLOCK_POLYGON),
                ))
                .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(
            read_last_balance(&mut chain, target),
            U256::from(200),
            "polygon balance must be unaffected by ethereum deal"
        );

        // Switch back to Ethereum: deal mutation must remain
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkCall::new((URL_ETH.to_string(), U256::from(BLOCK_ETH)))
                    .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionReadBridgeCall::new(()).abi_encode(),
        ))];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(
            read_last_balance(&mut chain, target),
            U256::from(999),
            "ethereum deal mutation must persist after switching back"
        );
    }

    /// Mid-transaction `vm.fork` switch must keep a prior `vm.store` on the
    /// previous fork and must not leak it onto the destination fork.
    ///
    /// Flow (single transaction via `actionForkStoreThenSwitch`):
    /// 1. fork(Ethereum)
    /// 2. store bridge slot0 = 99
    /// 3. load confirms 99 (same-tx journal is visible -> `lastSlot0`)
    /// 4. fork(Polygon)  // switch mid-tx; leave active on Polygon
    ///
    /// Then in a later transaction:
    /// 5. fork(Ethereum) again -> slot0 must still be 99
    /// 6. fork(Polygon) -> slot0 must stay at remote original 2
    #[test]
    fn mid_transaction_fork_switch_preserves_prior_fork_store() {
        let transport = MockTransport::default();
        setup_eth_polygon_forks(&transport);
        let (mut chain, target) = deploy_with_transport(transport);

        let mutated = alloy_primitives::FixedBytes::from(U256::from(99).to_be_bytes());
        let polygon_remote = alloy_primitives::FixedBytes::from(U256::from(2).to_be_bytes());

        // One transaction: store on Ethereum, then switch to Polygon.
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkStoreThenSwitchCall::new((
                    URL_ETH.to_string(),
                    U256::from(BLOCK_ETH),
                    mutated,
                    URL_POLYGON.to_string(),
                    U256::from(BLOCK_POLYGON),
                ))
                .abi_encode(),
            )),
        ];
        assert!(
            chain.exec(&txs).unwrap().results[0].success,
            "mid-tx store+switch must succeed"
        );

        // Same-tx load before the switch was recorded into lastSlot0.
        assert_eq!(
            read_last_slot0(&mut chain, target),
            mutated,
            "store must be visible in the same transaction before the fork switch"
        );

        // Active fork is Polygon after the mid-tx switch.
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::getChainIdCall::new(()).abi_encode(),
        ))];
        let execution = chain.exec(&txs).unwrap();
        assert!(execution.results[0].success);
        let chain_id = ForkHarness::getChainIdCall::abi_decode_returns(
            &execution.results[0].output.clone().unwrap(),
        )
        .unwrap();
        assert_eq!(
            chain_id,
            U256::from(0x89),
            "active fork after mid-tx switch must be polygon"
        );

        // Switch back to Ethereum in a new transaction and re-read.
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkCall::new((URL_ETH.to_string(), U256::from(BLOCK_ETH)))
                    .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionReadBridgeCall::new(()).abi_encode(),
        ))];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        assert_eq!(
            read_last_slot0(&mut chain, target),
            mutated,
            "ethereum store must survive mid-tx fork switch"
        );

        // Polygon must keep its remote original (no leak from ethereum store).
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkAndReadBridgeCall::new((
                    URL_POLYGON.to_string(),
                    U256::from(BLOCK_POLYGON),
                ))
                .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);
        assert_eq!(
            read_last_slot0(&mut chain, target),
            polygon_remote,
            "polygon must not receive ethereum mid-tx store"
        );
    }

    /// Mid-transaction `vm.fork` switch must keep a prior `vm.deal` on the
    /// previous fork.
    #[test]
    fn mid_transaction_fork_switch_preserves_prior_fork_deal() {
        let transport = MockTransport::default();
        setup_eth_polygon_forks(&transport);
        let (mut chain, target) = deploy_with_transport(transport);

        let mutated = U256::from(999);

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkDealThenSwitchCall::new((
                    URL_ETH.to_string(),
                    U256::from(BLOCK_ETH),
                    mutated,
                    URL_POLYGON.to_string(),
                    U256::from(BLOCK_POLYGON),
                ))
                .abi_encode(),
            )),
        ];
        assert!(
            chain.exec(&txs).unwrap().results[0].success,
            "mid-tx deal+switch must succeed"
        );

        assert_eq!(
            read_last_balance(&mut chain, target),
            mutated,
            "deal must be visible in the same transaction before the fork switch"
        );

        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                ForkHarness::actionForkCall::new((URL_ETH.to_string(), U256::from(BLOCK_ETH)))
                    .abi_encode(),
            )),
        ];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            ForkHarness::actionReadBridgeCall::new(()).abi_encode(),
        ))];
        assert!(chain.exec(&txs).unwrap().results[0].success);

        assert_eq!(
            read_last_balance(&mut chain, target),
            mutated,
            "ethereum deal must survive mid-tx fork switch"
        );
    }

    /// Coverage collection under multi-fork must still merge by bytecode hash,
    /// not by address. Running the same harness path on two forks must not
    /// double-count contracts.
    #[test]
    fn multi_fork_coverage_keys_by_bytecode_not_address() {
        let transport = MockTransport::default();
        setup_eth_polygon_forks(&transport);

        let initcode = load_initcode("ForkHarness.sol:ForkHarness");
        let config = ChainConfig::default()
            .coverage(true)
            .with_transport(Arc::new(transport))
            .with_fork_defaults(ForkDBConfig::new(""));
        let mut chain = Chain::new(config).unwrap();
        let deployment = chain.deploy(DeployInput::new(&initcode)).unwrap();
        assert!(deployment.result.success);
        let target = deployment.address.unwrap();
        let setup = chain.setup(SetupInput::new(target)).unwrap();
        assert!(setup.result.success);

        let shared = SharedCoverage::new();
        shared.merge(&deployment.coverage);
        shared.merge(&setup.coverage);

        // Execute the same harness bytecode path on both forks.
        for (url, block) in [(URL_ETH, BLOCK_ETH), (URL_POLYGON, BLOCK_POLYGON)] {
            let txs = vec![
                Transaction::new(target).calldata(Bytes::from(
                    ForkHarness::actionForkAndReadBridgeCall::new((
                        url.to_string(),
                        U256::from(block),
                    ))
                    .abi_encode(),
                )),
            ];
            let execution = chain.exec(&txs).unwrap();
            assert!(execution.results[0].success);
            if let Some(cov) = execution.coverage.as_ref() {
                shared.merge(cov);
            }
        }

        // Harness runtime bytecode is one contract; both forks share it.
        // Bridge has empty code so it does not add a coverage contract.
        assert!(
            shared.contract_count() >= 1,
            "must record at least the harness contract"
        );
        // Running the same harness on two forks must not invent a second
        // harness contract id (coverage is code-hash keyed).
        let contracts_after_first = {
            let transport2 = MockTransport::default();
            setup_eth_polygon_forks(&transport2);
            let config2 = ChainConfig::default()
                .coverage(true)
                .with_transport(Arc::new(transport2))
                .with_fork_defaults(ForkDBConfig::new(""));
            let mut chain2 = Chain::new(config2).unwrap();
            let deployment2 = chain2.deploy(DeployInput::new(&initcode)).unwrap();
            let target2 = deployment2.address.unwrap();
            let setup2 = chain2.setup(SetupInput::new(target2)).unwrap();
            let shared2 = SharedCoverage::new();
            shared2.merge(&deployment2.coverage);
            shared2.merge(&setup2.coverage);
            let txs = vec![
                Transaction::new(target2).calldata(Bytes::from(
                    ForkHarness::actionForkAndReadBridgeCall::new((
                        URL_ETH.to_string(),
                        U256::from(BLOCK_ETH),
                    ))
                    .abi_encode(),
                )),
            ];
            let execution = chain2.exec(&txs).unwrap();
            if let Some(cov) = execution.coverage.as_ref() {
                shared2.merge(cov);
            }
            shared2.contract_count()
        };

        assert_eq!(
            shared.contract_count(),
            contracts_after_first,
            "second fork must not add another harness contract id"
        );
    }
}
