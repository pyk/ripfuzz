//! Fuzzer factory: constructs per-thread fuzzer engines.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy_primitives::{Address, U256};
use anyhow::{Context as _, Result};
use revm::{
    MainBuilder, MainContext,
    context::{Context, TxEnv},
    inspector::InspectCommitEvm,
    primitives::{Bytes, TxKind},
};
use tracing::info;

use crate::corpus::{Call, CallMeta, Corpus, CorpusItem};
use crate::coverage::LocalCoverage;
use crate::evm;
use crate::evm::cheatcode;
use crate::fuzzer::config::FuzzerConfig;
use crate::fuzzer::mutators::Mutator;
use crate::fuzzer::{Crash, FuzzerEngine, FuzzerResult};
use crate::target;

/// Options for configuring a [`Factory`].
#[derive(Debug, Clone, Copy)]
pub struct FactoryOptions {
    pub seed: u64,
    pub sequence_length: usize,
    pub max_block_number_delay: u64,
    pub max_block_timestamp_delay: u64,
    pub caller: Address,
}

impl FactoryOptions {
    pub fn new() -> Self {
        Self {
            seed: 0,
            sequence_length: 32,
            max_block_number_delay: 5,
            max_block_timestamp_delay: 5,
            caller: evm::chain::DEFAULT_DEPLOYER,
        }
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn sequence_length(mut self, len: usize) -> Self {
        self.sequence_length = len;
        self
    }

    pub fn max_block_number_delay(mut self, delay: u64) -> Self {
        self.max_block_number_delay = delay;
        self
    }

    pub fn max_block_timestamp_delay(mut self, delay: u64) -> Self {
        self.max_block_timestamp_delay = delay;
        self
    }

    pub fn caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }
}

impl Default for FactoryOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Something that can construct per-thread fuzzer engines.
pub trait Factory: Send + Sync + std::fmt::Debug + 'static {
    fn create(
        &self,
        contract: Arc<target::Contract>,
        chain: evm::Chain,
        deployed_address: Address,
        config: FuzzerConfig,
        fuzzed_selectors: Arc<Vec<[u8; 4]>>,
    ) -> Box<dyn FuzzerEngine>;
}

/// The default fuzzer factory that produces [`Engine`] instances.
#[derive(Debug, Clone)]
pub struct DefaultFactory {
    pub options: FactoryOptions,
}

impl DefaultFactory {
    pub fn new(options: FactoryOptions) -> Self {
        Self { options }
    }
}

impl Factory for DefaultFactory {
    fn create(
        &self,
        contract: Arc<target::Contract>,
        chain: evm::Chain,
        deployed_address: Address,
        config: FuzzerConfig,
        fuzzed_selectors: Arc<Vec<[u8; 4]>>,
    ) -> Box<dyn FuzzerEngine> {
        Box::new(Engine::new(
            contract,
            chain,
            deployed_address,
            config,
            fuzzed_selectors,
            self.options.caller,
        ))
    }
}

/// Details about an assert panic crash detected during execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashInfo {
    pub name: String,
    pub selector: [u8; 4],
}

/// Outcome of executing a single call sequence.
#[derive(Debug, Clone, Default)]
pub struct ExecutionOutcome {
    pub coverage: LocalCoverage,
    pub call_meta: Vec<CallMeta>,
    pub all_ok: bool,
    pub total_calls: u64,
    pub total_gas: u64,
    pub crash: Option<CrashInfo>,
}

/// EVM-powered fuzzer engine that executes call sequences against a cloned
/// [`evm::Chain`] and a deployed [`target::Contract`].
#[derive(Debug)]
pub struct Engine {
    contract: Arc<target::Contract>,
    chain: evm::Chain,
    deployed_address: Address,
    config: FuzzerConfig,
    fuzzed_selectors: Arc<Vec<[u8; 4]>>,
    caller: Address,
}

impl Engine {
    pub fn new(
        contract: Arc<target::Contract>,
        chain: evm::Chain,
        deployed_address: Address,
        config: FuzzerConfig,
        fuzzed_selectors: Arc<Vec<[u8; 4]>>,
        caller: Address,
    ) -> Self {
        Self {
            contract,
            chain,
            deployed_address,
            config,
            fuzzed_selectors,
            caller,
        }
    }

    fn execute_sequence(&self, calls: &[Call]) -> Result<ExecutionOutcome> {
        let mut chain = self.chain.clone();
        let db = chain.database.take().context("database unavailable")?;
        let mut ctx = Context::mainnet().with_db(db);
        ctx.block = chain.block_env.clone();
        ctx.cfg = chain.cfg_env.clone();
        ctx.cfg.disable_balance_check = true;
        ctx.cfg.tx_gas_limit_cap = Some(u64::MAX);

        let inspector = cheatcode::Inspector::default();
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        let mut total_calls = 0u64;
        let mut total_gas = 0u64;
        let mut all_ok = true;
        let mut call_meta = Vec::new();
        let mut crash = None;

        for call in calls {
            let current_number = u64::try_from(evm.ctx.block.number).unwrap_or(u64::MAX);
            let current_timestamp = u64::try_from(evm.ctx.block.timestamp).unwrap_or(u64::MAX);
            let new_number = current_number.saturating_add(call.block_number_delay);
            let new_timestamp = current_timestamp.saturating_add(call.block_timestamp_delay);
            evm.ctx.block.number = U256::from(new_number);
            evm.ctx.block.timestamp = U256::from(new_timestamp);

            let tx_origin = evm.inspector.state.prank.origin_for_top_level(self.caller);

            let tx = TxEnv {
                caller: tx_origin,
                kind: TxKind::Call(self.deployed_address),
                data: Bytes::from(call.encode()),
                gas_limit: u64::MAX,
                value: U256::ZERO,
                ..Default::default()
            };

            let result = evm
                .inspect_tx_commit(tx)
                .context("revm transaction failed")?;

            total_calls += 1;
            let gas_used = result.tx_gas_used();
            total_gas += gas_used;

            let success = result.is_success();
            let reason = if !success {
                match &result {
                    revm::context::result::ExecutionResult::Halt { reason, .. } => {
                        Some(format!("halted: {reason}"))
                    }
                    revm::context::result::ExecutionResult::Revert { .. } => {
                        Some("reverted".into())
                    }
                    _ => Some("failed".into()),
                }
            } else {
                None
            };

            call_meta.push(CallMeta {
                block_number: new_number,
                block_timestamp: new_timestamp,
                gas_used,
                success,
                reason,
            });

            if !success {
                if let Some(output) = result.output()
                    && is_assert_failure(output)
                {
                    let name = self
                        .contract
                        .abi
                        .functions()
                        .find(|f| f.selector().as_slice() == call.selector)
                        .map(|f| f.name.to_owned())
                        .unwrap_or_else(|| format!("0x{}", hex::encode(call.selector)));
                    crash = Some(CrashInfo {
                        name,
                        selector: call.selector,
                    });
                }
                all_ok = false;
                break;
            }
        }

        // Check invariants
        if all_ok {
            for inv in &self.contract.invariant_functions {
                let tx_origin = evm.inspector.state.prank.origin_for_top_level(self.caller);
                let tx = TxEnv {
                    caller: tx_origin,
                    kind: TxKind::Call(self.deployed_address),
                    data: Bytes::from(inv.selector().as_slice().to_vec()),
                    gas_limit: u64::MAX,
                    value: U256::ZERO,
                    ..Default::default()
                };
                let result = evm
                    .inspect_tx_commit(tx)
                    .context("revm transaction failed")?;

                total_calls += 1;
                let gas_used = result.tx_gas_used();
                total_gas += gas_used;

                let success = result.is_success();
                let reason = if !success {
                    match &result {
                        revm::context::result::ExecutionResult::Halt { reason, .. } => {
                            Some(format!("halted: {reason}"))
                        }
                        revm::context::result::ExecutionResult::Revert { .. } => {
                            Some("reverted".into())
                        }
                        _ => Some("failed".into()),
                    }
                } else {
                    None
                };

                call_meta.push(CallMeta {
                    block_number: u64::try_from(evm.ctx.block.number).unwrap_or(u64::MAX),
                    block_timestamp: u64::try_from(evm.ctx.block.timestamp).unwrap_or(u64::MAX),
                    gas_used,
                    success,
                    reason: reason.to_owned(),
                });

                if !success {
                    if let Some(output) = result.output()
                        && is_assert_failure(output)
                    {
                        crash = Some(CrashInfo {
                            name: inv.name.to_owned(),
                            selector: inv.selector().into(),
                        });
                    }
                    all_ok = false;
                    break;
                }
            }
        }

        chain.database = Some(evm.ctx.journaled_state.database);

        Ok(ExecutionOutcome {
            coverage: LocalCoverage::default(),
            call_meta,
            all_ok,
            total_calls,
            total_gas,
            crash,
        })
    }

    fn mutate_corpus_item(
        &self,
        corpus: &Arc<std::sync::RwLock<Corpus>>,
        mutators: &mut [Box<dyn Mutator>],
        rng: &mut fastrand::Rng,
        idx: usize,
        base: CorpusItem,
    ) -> (Vec<Call>, crate::fuzzer::mutators::MutationResult) {
        let mut calls = base.calls;
        let idx_mut = rng.usize(0..mutators.len());
        let m = &mut mutators[idx_mut];
        let result = m.mutate(rng, &mut calls);
        if result == crate::fuzzer::mutators::MutationResult::Mutated
            && let Ok(mut c) = corpus.write()
            && let Some(base_item) = c.items.get_mut(idx)
        {
            base_item.total_mutations += 1;
        }
        (calls, result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &self,
        corpus: Arc<std::sync::RwLock<Corpus>>,
        max_runs: u64,
        fuzzer_id: usize,
        start: Instant,
        timeout: Option<Duration>,
        shared_runs: Arc<AtomicU64>,
        shared_calls: Arc<AtomicU64>,
        shared_gas: Arc<AtomicU64>,
        shared_failures: Arc<AtomicU64>,
    ) -> Result<FuzzerResult> {
        info!(max_runs, fuzzer_id, "fuzzer run starting");

        let mut rng = fastrand::Rng::with_seed(self.config.seed + fuzzer_id as u64);
        let mut failures = Vec::new();

        let mut mutators: Vec<Box<dyn Mutator>> = vec![
            Box::new(crate::fuzzer::mutators::SequenceSwapMutator),
            Box::new(crate::fuzzer::mutators::SequenceInsertMutator::new(
                self.fuzzed_selectors.to_vec(),
                self.config.max_block_number_delay,
                self.config.max_block_timestamp_delay,
            )),
            Box::new(crate::fuzzer::mutators::SequenceDeleteMutator),
            Box::new(crate::fuzzer::mutators::SequenceSpliceMutator::new(
                corpus.clone(),
            )),
            Box::new(crate::fuzzer::mutators::SequenceInterleaveMutator::new(
                corpus.clone(),
            )),
            Box::new(crate::fuzzer::mutators::SequenceHeadMutator::new(
                corpus.clone(),
            )),
            Box::new(crate::fuzzer::mutators::SequenceTailMutator::new(
                corpus.clone(),
            )),
            Box::new(crate::fuzzer::mutators::SequenceArgMutator::new(
                self.contract.abi.clone(),
            )),
            Box::new(crate::fuzzer::mutators::SequenceDelayMutator::new(
                self.config.max_block_number_delay,
                self.config.max_block_timestamp_delay,
            )),
        ];

        let mut runs = 0u64;
        let mut total_calls = 0u64;
        let mut total_gas = 0u64;

        for _ in 0..max_runs {
            if let Some(timeout) = timeout
                && start.elapsed() > timeout
            {
                break;
            }

            let item = {
                let Ok(mut corpus_guard) = corpus.write() else {
                    break;
                };
                corpus_guard.pop_pending_item()
            };

            let is_replay = item.is_some();
            let mut base_idx = None;
            let calls = if let Some(item) = item {
                item.calls
            } else {
                let has_entries = if let Ok(c) = corpus.read() {
                    c.has_entries()
                } else {
                    false
                };
                if rng.bool() && has_entries {
                    let picked = if let Ok(c) = corpus.read() {
                        c.random_item_for_mutation_with_index(&mut rng)
                    } else {
                        None
                    };
                    if let Some((idx, base)) = picked {
                        base_idx = Some(idx);
                        let (calls, _) =
                            self.mutate_corpus_item(&corpus, &mut mutators, &mut rng, idx, base);
                        calls
                    } else {
                        crate::fuzzer::generate_random_sequence(
                            &self.fuzzed_selectors,
                            &mut rng,
                            &self.config,
                        )
                    }
                } else {
                    crate::fuzzer::generate_random_sequence(
                        &self.fuzzed_selectors,
                        &mut rng,
                        &self.config,
                    )
                }
            };

            let outcome = self.execute_sequence(&calls)?;
            total_calls += outcome.total_calls;
            total_gas += outcome.total_gas;
            let all_ok = outcome.all_ok;
            let local_coverage = outcome.coverage;

            let mut item = CorpusItem::new(calls);
            if all_ok {
                let Ok(mut corpus_guard) = corpus.write() else {
                    continue;
                };
                let added = corpus_guard.check_and_update_coverage(&local_coverage, &item);
                if added
                    && let Some(idx) = base_idx
                    && let Some(base_item) = corpus_guard.items.get_mut(idx)
                {
                    base_item.new_finds_produced += 1;
                }
                if is_replay && !added {
                    corpus_guard.add_item_for_mutation(&item);
                }
            }

            if let Some(crash) = outcome.crash {
                let call_sequence = std::mem::take(&mut item.calls);
                failures.push(Crash {
                    function_name: crash.name,
                    selector: crash.selector,
                    call_sequence,
                    call_meta: outcome.call_meta,
                });
                shared_failures.fetch_add(1, Ordering::Relaxed);
            }

            runs += 1;
            shared_runs.fetch_add(1, Ordering::Relaxed);
            shared_calls.fetch_add(outcome.total_calls, Ordering::Relaxed);
            shared_gas.fetch_add(outcome.total_gas, Ordering::Relaxed);
        }

        // Sync discovered failures into the shared corpus for persistence.
        if let Ok(mut c) = corpus.write() {
            for failure in &failures {
                c.add_failure(CorpusItem::new(failure.call_sequence.as_slice().to_vec()));
            }
        }

        info!(runs, fuzzer_id, "fuzzer run finished");
        Ok(FuzzerResult {
            runs,
            failures,
            total_calls,
            total_gas,
        })
    }
}

impl FuzzerEngine for Engine {
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        corpus: Arc<std::sync::RwLock<Corpus>>,
        max_runs: u64,
        fuzzer_id: usize,
        start: Instant,
        timeout: Option<Duration>,
        shared_runs: Arc<AtomicU64>,
        shared_calls: Arc<AtomicU64>,
        shared_gas: Arc<AtomicU64>,
        shared_failures: Arc<AtomicU64>,
    ) -> Result<FuzzerResult> {
        self.run(
            corpus,
            max_runs,
            fuzzer_id,
            start,
            timeout,
            shared_runs,
            shared_calls,
            shared_gas,
            shared_failures,
        )
    }
}

/// Solidity `Panic(uint256)` selector: keccak256("Panic(uint256)")[:4]
const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];

/// Detect a Solidity `assert` failure (`Panic(0x01)`) in revert output.
fn is_assert_failure(output: &Bytes) -> bool {
    output.len() >= 36 && output[..4] == PANIC_SELECTOR && output[35] == 0x01
}
