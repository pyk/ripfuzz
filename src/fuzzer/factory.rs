//! Fuzzer factory: owns the chain and creates per-thread [`Fuzzer`] instances.

use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy_primitives::Address;
use anyhow::Result;
use tracing::{info, instrument};

use crate::corpus::{Call, CorpusItem};
use crate::evm;
use crate::fuzzer::config::Config;
use crate::fuzzer::corpus::SharedCorpus;
use crate::fuzzer::engine;
use crate::fuzzer::metrics::SharedMetrics;
use crate::fuzzer::mutators::Mutator;
use crate::target;

/// Result produced by a single fuzzer thread.
#[derive(Debug, Clone)]
pub struct FuzzerResult {
    pub runs: u64,
    pub failures: Vec<Crash>,
    pub total_calls: u64,
    pub total_gas: u64,
}

/// A single crash (assert panic) discovered during fuzzing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Crash {
    pub function_name: String,
    pub selector: [u8; 4],
    pub call_sequence: Vec<Call>,
    pub call_meta: Vec<crate::corpus::CallMeta>,
}

/// Factory that owns the base chain state and spawns [`Fuzzer`] instances.
///
/// The factory holds the post-deployment, post-setup chain snapshot.
/// Each [`Fuzzer`] receives an independent clone of this snapshot so
/// sequences execute against isolated state.
#[derive(Debug, Clone)]
pub struct Factory {
    chain: evm::Chain,
    contract: Arc<target::Contract>,
    deployed_address: Address,
    config: Config,
    fuzzed_selectors: Arc<Vec<[u8; 4]>>,
    caller: Address,
    corpus: SharedCorpus,
    metrics: SharedMetrics,
}

impl Factory {
    /// Create a new factory.
    pub fn new(
        chain: evm::Chain,
        contract: target::Contract,
        deployed_address: Address,
        config: Config,
        corpus: SharedCorpus,
    ) -> Self {
        let fuzzed_selectors: Vec<[u8; 4]> = contract
            .target_functions
            .iter()
            .map(|f| f.selector().into())
            .collect();

        Self {
            chain,
            contract: Arc::new(contract),
            deployed_address,
            config,
            fuzzed_selectors: Arc::new(fuzzed_selectors),
            caller: evm::chain::DEFAULT_DEPLOYER,
            corpus,
            metrics: SharedMetrics::new(),
        }
    }

    /// Set the default caller address used for fuzz transactions.
    pub fn with_caller(mut self, caller: Address) -> Self {
        self.caller = caller;
        self
    }

    /// Provide shared metrics.
    pub fn with_metrics(mut self, metrics: SharedMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Access the shared corpus.
    pub fn corpus(&self) -> &SharedCorpus {
        &self.corpus
    }

    /// Access the shared metrics.
    pub fn metrics(&self) -> &SharedMetrics {
        &self.metrics
    }

    /// Validate all pending corpus items by replaying them.
    pub fn validate_corpus(&self) {
        let chain = self.chain.clone();
        let contract = Arc::clone(&self.contract);
        let deployed_address = self.deployed_address;
        let caller = self.caller;

        self.corpus.validate_pending(|calls| {
            engine::execute_sequence(&chain, &contract, deployed_address, caller, calls)
                .unwrap_or_default()
        });
    }

    /// Create a new [`Fuzzer`] for the given thread id.
    pub fn create(&self, fuzzer_id: usize) -> Fuzzer {
        let corpus_arc = self.corpus.to_arc();
        let mutators: Vec<Box<dyn Mutator>> = vec![
            Box::new(crate::fuzzer::mutators::SequenceSwapMutator),
            Box::new(crate::fuzzer::mutators::SequenceInsertMutator::new(
                self.fuzzed_selectors.to_vec(),
                self.config.max_block_number_delay,
                self.config.max_block_timestamp_delay,
            )),
            Box::new(crate::fuzzer::mutators::SequenceDeleteMutator),
            Box::new(crate::fuzzer::mutators::SequenceSpliceMutator::new(
                Arc::clone(&corpus_arc),
            )),
            Box::new(crate::fuzzer::mutators::SequenceInterleaveMutator::new(
                Arc::clone(&corpus_arc),
            )),
            Box::new(crate::fuzzer::mutators::SequenceHeadMutator::new(
                Arc::clone(&corpus_arc),
            )),
            Box::new(crate::fuzzer::mutators::SequenceTailMutator::new(
                Arc::clone(&corpus_arc),
            )),
            Box::new(crate::fuzzer::mutators::SequenceArgMutator::new(
                self.contract.abi.clone(),
            )),
            Box::new(crate::fuzzer::mutators::SequenceDelayMutator::new(
                self.config.max_block_number_delay,
                self.config.max_block_timestamp_delay,
            )),
        ];

        Fuzzer {
            chain: self.chain.clone(),
            contract: Arc::clone(&self.contract),
            deployed_address: self.deployed_address,
            config: Config {
                seed: self.config.seed.wrapping_add(fuzzer_id as u64),
                ..self.config
            },
            fuzzed_selectors: Arc::clone(&self.fuzzed_selectors),
            caller: self.caller,
            corpus: self.corpus.clone(),
            metrics: self.metrics.clone(),
            mutators,
        }
    }
}

/// Per-thread fuzzer that executes call sequences and reports results.
///
/// Created by [`Factory::create`] and run via [`Fuzzer::run`].
pub struct Fuzzer {
    chain: evm::Chain,
    contract: Arc<target::Contract>,
    deployed_address: Address,
    config: Config,
    fuzzed_selectors: Arc<Vec<[u8; 4]>>,
    caller: Address,
    corpus: SharedCorpus,
    metrics: SharedMetrics,
    mutators: Vec<Box<dyn Mutator>>,
}

impl std::fmt::Debug for Fuzzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fuzzer")
            .field("chain", &self.chain)
            .field("contract", &self.contract)
            .field("deployed_address", &self.deployed_address)
            .field("config", &self.config)
            .field("fuzzed_selectors", &self.fuzzed_selectors)
            .field("caller", &self.caller)
            .field("corpus", &self.corpus)
            .field("metrics", &self.metrics)
            .field(
                "mutators",
                &format_args!("[{} mutators]", self.mutators.len()),
            )
            .finish()
    }
}

impl Fuzzer {
    /// Run the fuzzer for up to `max_runs` iterations.
    ///
    /// The fuzzer loop uses the shared corpus for mutation and the shared
    /// metrics for counters. It stops early if `timeout` is reached.
    #[instrument(skip(self), fields(max_runs))]
    pub fn run(&mut self, max_runs: u64, timeout: Option<Duration>) -> Result<FuzzerResult> {
        let start = Instant::now();
        let mut rng = fastrand::Rng::with_seed(self.config.seed);
        let mut local_failures = Vec::new();
        let mut runs = 0u64;
        let mut total_calls = 0u64;
        let mut total_gas = 0u64;

        for _ in 0..max_runs {
            if let Some(t) = timeout
                && start.elapsed() > t
            {
                break;
            }

            if let Some(snapshot) = self.metrics.maybe_print() {
                info!(
                    elapsed = ?snapshot.elapsed,
                    runs = snapshot.runs,
                    calls = snapshot.calls,
                    gas = snapshot.gas,
                    failures = snapshot.failures,
                    "fuzz metrics",
                );
            }

            let item = self.corpus.pop_pending();
            let is_replay = item.is_some();
            let mut base_idx = None;
            let calls = if let Some(item) = item {
                item.calls
            } else {
                let has_entries = self.corpus.has_entries();
                if rng.bool() && has_entries {
                    match self.corpus.pick_for_mutation(&mut rng) {
                        Some((idx, base)) => {
                            base_idx = Some(idx);
                            let mut calls = base.calls;
                            let m_idx = rng.usize(0..self.mutators.len());
                            let _ = self.mutators[m_idx].mutate(&mut rng, &mut calls);
                            calls
                        }
                        None => {
                            generate_random_sequence(&self.fuzzed_selectors, &mut rng, &self.config)
                        }
                    }
                } else {
                    generate_random_sequence(&self.fuzzed_selectors, &mut rng, &self.config)
                }
            };

            let outcome = engine::execute_sequence(
                &self.chain,
                &self.contract,
                self.deployed_address,
                self.caller,
                &calls,
            )?;

            total_calls += outcome.total_calls;
            total_gas += outcome.total_gas;
            self.metrics.record(outcome.total_calls, outcome.total_gas);

            if outcome.all_ok {
                // checkrs: allow(clone_in_loops)
                let item = CorpusItem::new(calls.clone());
                let added = self.corpus.record_interesting(item, &outcome.coverage);
                match (base_idx, added) {
                    (Some(idx), true) => {
                        self.corpus.record_new_find(idx);
                        self.corpus.record_mutation(idx);
                    }
                    (Some(idx), false) => {
                        self.corpus.record_mutation(idx);
                    }
                    (None, _) => {}
                }
                if is_replay && !added {
                    self.corpus
                        // checkrs: allow(clone_in_loops)
                        .add_item_for_mutation(CorpusItem::new(calls.clone()));
                }
            }

            if let Some(crash_info) = outcome.crash {
                local_failures.push(Crash {
                    function_name: crash_info.name,
                    selector: crash_info.selector,
                    call_sequence: calls,
                    call_meta: outcome.call_meta,
                });
                self.metrics.record_failure();
            }

            runs += 1;
        }

        // Sync failures into shared corpus for persistence.
        for failure in &local_failures {
            self.corpus
                // checkrs: allow(clone_in_loops)
                .record_failure(CorpusItem::new(failure.call_sequence.clone()));
        }

        info!(runs, "fuzzer run finished");
        Ok(FuzzerResult {
            runs,
            failures: local_failures,
            total_calls,
            total_gas,
        })
    }
}

/// Generate a random call sequence from the fuzzed selectors.
pub(crate) fn generate_random_sequence(
    selectors: &[[u8; 4]],
    rng: &mut fastrand::Rng,
    config: &Config,
) -> Vec<Call> {
    let len = rng.usize(1..=config.sequence_length.max(1));
    let mut calls = Vec::with_capacity(len);
    for _ in 0..len {
        if selectors.is_empty() {
            break;
        }
        let sel_idx = rng.usize(0..selectors.len());
        let mut call = Call {
            selector: selectors[sel_idx],
            args: vec![0u8; 32 * 3],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        };
        if config.max_block_number_delay > 0 {
            call.block_number_delay = rng.u64(0..config.max_block_number_delay + 1);
        }
        if config.max_block_timestamp_delay > 0 {
            call.block_timestamp_delay = rng.u64(0..config.max_block_timestamp_delay + 1);
        }
        call.cap_delays();
        calls.push(call);
    }
    calls
}

/// Format a crash's call sequence as a flat, Medusa-style log.
pub fn format_failure(
    contract: &target::Contract,
    failure: &Crash,
    sender: revm::primitives::Address,
) -> String {
    let mut lines = Vec::new();
    for (i, call) in failure.call_sequence.iter().enumerate() {
        let n = i + 1;

        let block = failure
            .call_meta
            .get(i)
            .map(|m| m.block_number)
            .unwrap_or(n as u64);
        let time = failure
            .call_meta
            .get(i)
            .map(|m| m.block_timestamp)
            .unwrap_or(n as u64);

        let func = contract
            .abi
            .functions()
            .find(|f| f.selector().as_slice() == call.selector);

        let func_name = if let Some(f) = func {
            f.name.to_owned()
        } else {
            format!("0x{}", hex::encode(call.selector))
        };

        let mut delay_suffix = String::new();
        if call.block_number_delay != 0 {
            delay_suffix.push_str(&format!(", block_number_delay={}", call.block_number_delay));
        }
        if call.block_timestamp_delay != 0 {
            delay_suffix.push_str(&format!(
                ", block_timestamp_delay={}",
                call.block_timestamp_delay
            ));
        }

        let args = if let Some(func_abi) = func {
            if call.args.is_empty() {
                "()".into()
            } else {
                let types_result = func_abi
                    .inputs
                    .iter()
                    .map(|p| p.selector_type().parse::<alloy_dyn_abi::DynSolType>())
                    .collect();
                let Ok(types) = types_result else {
                    let raw = format!("(0x{})", hex::encode(&call.args));
                    lines.push(format!(
                        "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
                        n,
                        contract.artifact_id.name,
                        func_name,
                        raw,
                        block,
                        time,
                        u64::MAX,
                        sender,
                        delay_suffix,
                    ));
                    continue;
                };

                let tuple = alloy_dyn_abi::DynSolType::Tuple(types);
                let Ok(decoded) = tuple.abi_decode_params(&call.args) else {
                    let raw = format!("(0x{})", hex::encode(&call.args));
                    lines.push(format!(
                        "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
                        n,
                        contract.artifact_id.name,
                        func_name,
                        raw,
                        block,
                        time,
                        u64::MAX,
                        sender,
                        delay_suffix,
                    ));
                    continue;
                };

                let values = match decoded {
                    alloy_dyn_abi::DynSolValue::Tuple(v) => v,
                    other => vec![other],
                };

                let args_str = values
                    .iter()
                    .map(format_dyn_value)
                    .collect::<Vec<String>>()
                    .join(", ");

                format!("({})", args_str)
            }
        } else {
            format!("0x{}", hex::encode(&call.args))
        };

        lines.push(format!(
            "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
            n,
            contract.artifact_id.name,
            func_name,
            args,
            block,
            time,
            u64::MAX,
            sender,
            delay_suffix,
        ));
    }
    lines.join("\n")
}

fn format_dyn_value(v: &alloy_dyn_abi::DynSolValue) -> String {
    match v {
        alloy_dyn_abi::DynSolValue::Bool(b) => format!("{}", b),
        alloy_dyn_abi::DynSolValue::Int(i, _) => format!("{}", i),
        alloy_dyn_abi::DynSolValue::Uint(u, _) => format!("{}", u),
        alloy_dyn_abi::DynSolValue::Address(a) => format!("{:?}", a),
        alloy_dyn_abi::DynSolValue::String(s) => format!("\"{}\"", s),
        alloy_dyn_abi::DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        alloy_dyn_abi::DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        _ => format!("{:?}", v),
    }
}
