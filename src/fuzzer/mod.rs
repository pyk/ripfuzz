//! Per-fuzzer instance that executes call sequences and reports results.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use alloy_dyn_abi::{DynSolType, DynSolValue};
use anyhow::Result;
use tracing::{info, instrument};

use crate::contract;
use crate::corpus::{Call, Corpus, CorpusItem};
use crate::fuzzer::config::FuzzerConfig;

pub mod config;
pub mod mutators;

/// Result produced by a single fuzzer.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct FuzzerResult {
    pub runs: u64,
    pub failures: Vec<Crash>,
    /// Total individual calls executed across all runs.
    pub total_calls: u64,
    /// Total gas consumed across all calls.
    pub total_gas: u64,
}

/// A single crash (assert panic) discovered during fuzzing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Crash {
    pub function_name: String,
    pub selector: [u8; 4],
    pub call_sequence: Vec<Call>,
    /// Per-call block number / timestamp captured during execution.
    pub call_meta: Vec<crate::chain::output::CallMeta>,
}

/// Format a crash's call sequence as a flat, Medusa-style log.
pub fn format_failure(
    artifact: &contract::ContractArtifact,
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

        let func = artifact
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
                    .map(|p| p.selector_type().parse::<DynSolType>())
                    .collect();
                let Ok(types) = types_result else {
                    let raw = format!("(0x{})", hex::encode(&call.args));
                    lines.push(format!(
                        "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
                        n,
                        artifact.contract_name,
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

                let tuple = DynSolType::Tuple(types);
                let Ok(decoded) = tuple.abi_decode_params(&call.args) else {
                    let raw = format!("(0x{})", hex::encode(&call.args));
                    lines.push(format!(
                        "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
                        n,
                        artifact.contract_name,
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
                    DynSolValue::Tuple(v) => v,
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
            artifact.contract_name,
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

/// Something that can execute a single fuzzer run and return the result.
pub trait FuzzerEngine: Send {
    /// Run the fuzzer against the shared corpus.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        corpus: Arc<RwLock<Corpus>>,
        max_runs: u64,
        fuzzer_id: usize,
        start: Instant,
        timeout: Option<Duration>,
        shared_runs: Arc<AtomicU64>,
        shared_calls: Arc<AtomicU64>,
        shared_gas: Arc<AtomicU64>,
        shared_failures: Arc<AtomicU64>,
    ) -> Result<FuzzerResult>;
}

/// Factory for constructing [`FuzzerEngine`] instances.
pub trait FuzzerFactory: Send + Sync + std::fmt::Debug + 'static {
    /// Create a new fuzzer engine for a given thread.
    fn create(
        &self,
        artifact: Arc<contract::ContractArtifact>,
        executor: Arc<dyn crate::chain::SequenceExecutor>,
        config: FuzzerConfig,
        fuzzed_selectors: Arc<Vec<[u8; 4]>>,
    ) -> Box<dyn FuzzerEngine>;
}

/// The default fuzzer factory that produces [`Fuzzer`] instances.
#[derive(Debug, Clone)]
pub struct DefaultFuzzerFactory;

impl FuzzerFactory for DefaultFuzzerFactory {
    fn create(
        &self,
        artifact: Arc<contract::ContractArtifact>,
        executor: Arc<dyn crate::chain::SequenceExecutor>,
        config: FuzzerConfig,
        fuzzed_selectors: Arc<Vec<[u8; 4]>>,
    ) -> Box<dyn FuzzerEngine> {
        Box::new(Fuzzer::new(artifact, executor, config, fuzzed_selectors))
    }
}

pub struct Fuzzer {
    artifact: Arc<contract::ContractArtifact>,
    executor: Arc<dyn crate::chain::SequenceExecutor>,
    fuzzed_selectors: Arc<Vec<[u8; 4]>>,
    config: FuzzerConfig,
}

impl Fuzzer {
    pub fn new(
        artifact: Arc<contract::ContractArtifact>,
        executor: Arc<dyn crate::chain::SequenceExecutor>,
        config: FuzzerConfig,
        fuzzed_selectors: Arc<Vec<[u8; 4]>>,
    ) -> Self {
        Self {
            artifact,
            executor,
            fuzzed_selectors,
            config,
        }
    }

    fn mutate_corpus_item(
        &self,
        corpus: &Arc<RwLock<Corpus>>,
        mutators: &mut [Box<dyn mutators::Mutator>],
        rng: &mut fastrand::Rng,
        idx: usize,
        base: CorpusItem,
    ) -> (Vec<Call>, mutators::MutationResult) {
        let mut calls = base.calls;
        let idx_mut = rng.usize(0..mutators.len());
        let m = &mut mutators[idx_mut];
        let result = m.mutate(rng, &mut calls);
        if result == mutators::MutationResult::Mutated
            && let Ok(mut c) = corpus.write()
            && let Some(base_item) = c.items.get_mut(idx)
        {
            base_item.total_mutations += 1;
        }
        (calls, result)
    }

    #[instrument(skip(self, corpus), fields(max_runs))]
    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &self,
        corpus: Arc<RwLock<Corpus>>,
        max_runs: u64,
        fuzzer_id: usize,
        start: std::time::Instant,
        timeout: Option<std::time::Duration>,
        shared_runs: Arc<AtomicU64>,
        shared_calls: Arc<AtomicU64>,
        shared_gas: Arc<AtomicU64>,
        shared_failures: Arc<AtomicU64>,
    ) -> Result<FuzzerResult> {
        info!(max_runs, fuzzer_id, "fuzzer run starting");

        let mut rng = fastrand::Rng::with_seed(self.config.seed + fuzzer_id as u64);
        let mut failures = Vec::new();

        let mut mutators: Vec<Box<dyn mutators::Mutator>> = vec![
            Box::new(mutators::SequenceSwapMutator),
            Box::new(mutators::SequenceInsertMutator::new(
                self.fuzzed_selectors.to_vec(),
                self.config.max_block_number_delay,
                self.config.max_block_timestamp_delay,
            )),
            Box::new(mutators::SequenceDeleteMutator),
            Box::new(mutators::SequenceSpliceMutator::new(corpus.clone())),
            Box::new(mutators::SequenceInterleaveMutator::new(corpus.clone())),
            Box::new(mutators::SequenceHeadMutator::new(corpus.clone())),
            Box::new(mutators::SequenceTailMutator::new(corpus.clone())),
            Box::new(mutators::SequenceArgMutator::new(self.artifact.abi.clone())),
            Box::new(mutators::SequenceDelayMutator::new(
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
                        generate_random_sequence(&self.fuzzed_selectors, &mut rng, &self.config)
                    }
                } else {
                    generate_random_sequence(&self.fuzzed_selectors, &mut rng, &self.config)
                }
            };

            let output = self.executor.execute(&calls)?;
            total_calls += output.total_calls;
            total_gas += output.total_gas;
            let all_ok = output.all_ok;
            let local_coverage = output.coverage;

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

            if let Some(crash) = output.crash {
                let call_sequence = std::mem::take(&mut item.calls);
                failures.push(Crash {
                    function_name: crash.name,
                    selector: crash.selector,
                    call_sequence,
                    call_meta: output.call_meta,
                });
                shared_failures.fetch_add(1, Ordering::Relaxed);
            }

            runs += 1;
            shared_runs.fetch_add(1, Ordering::Relaxed);
            shared_calls.fetch_add(output.total_calls, Ordering::Relaxed);
            shared_gas.fetch_add(output.total_gas, Ordering::Relaxed);
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

impl FuzzerEngine for Fuzzer {
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        corpus: Arc<RwLock<Corpus>>,
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

fn generate_random_sequence(
    selectors: &[[u8; 4]],
    rng: &mut fastrand::Rng,
    config: &FuzzerConfig,
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

#[cfg(test)]
mod tests {

    use crate::chain::output::CallMeta;
    use crate::contract;
    use crate::corpus;
    use crate::fuzzer::Crash;
    use crate::fuzzer::format_failure;

    #[test]
    fn format_failure_uses_block_number_and_timestamp_labels() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/challenges", "src/L1SimpleKnob.sol")
                .unwrap();

        let calls = vec![
            corpus::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
            corpus::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 3,
                block_timestamp_delay: 4,
                ..Default::default()
            },
            corpus::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            },
        ];

        let failure = Crash {
            function_name: "invariant_caught".into(),
            selector: [0; 4],
            call_sequence: calls,
            call_meta: vec![
                CallMeta {
                    block_number: 0,
                    block_timestamp: 0,
                    ..Default::default()
                },
                CallMeta {
                    block_number: 3,
                    block_timestamp: 4,
                    ..Default::default()
                },
                CallMeta {
                    block_number: 4,
                    block_timestamp: 5,
                    ..Default::default()
                },
            ],
        };

        let output = format_failure(&artifact, &failure, crate::chain::init::CALLER);
        assert!(
            output.contains("block_number="),
            "output should use block_number label:\n{}",
            output
        );
        assert!(
            output.contains("block_timestamp="),
            "output should use block_timestamp label:\n{}",
            output
        );
        assert!(
            !output.contains("block=0") && !output.contains("block=3"),
            "output should not use old block= label:\n{}",
            output
        );
        assert!(
            !output.contains("time=1") && !output.contains("time=5"),
            "output should not use old time= label:\n{}",
            output
        );
        assert!(
            output.contains("block_number_delay=3"),
            "output should show block_number_delay:\n{}",
            output
        );
        assert!(
            output.contains("block_timestamp_delay=4"),
            "output should show block_timestamp_delay:\n{}",
            output
        );
    }
}
