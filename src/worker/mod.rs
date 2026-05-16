//! Per-process fuzzing worker that executes the LibAFL loop.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use anyhow::{Context, Result};
use libafl::{
    corpus::ondisk::OnDiskMetadataFormat,
    corpus::{Corpus, InMemoryOnDiskCorpus, OnDiskCorpus, Testcase},
    executors::InProcessExecutor,
    feedbacks::{CrashFeedback, MaxMapFeedback},
    fuzzer::{Fuzzer as LibAflFuzzer, StdFuzzer},
    observers::StdMapObserver,
    schedulers::QueueScheduler,
    stages::StdMutationalStage,
    state::{HasCorpus, StdState},
};
use libafl_bolts::{rands::StdRand, tuples::tuple_list};
use tracing::{debug, info, instrument, trace, warn};

use crate::campaign::CampaignConfig;
use crate::contract;
use crate::corpus;
use crate::evm;
use crate::inspector;

pub mod mutators;

pub(crate) type MyCorpus = InMemoryOnDiskCorpus<corpus::CallSequenceInput>;
pub(crate) type MyObjectiveCorpus = OnDiskCorpus<corpus::CallSequenceInput>;
pub(crate) type MyState = StdState<MyCorpus, corpus::CallSequenceInput, StdRand, MyObjectiveCorpus>;
pub(crate) type MyShMem = libafl_bolts::shmem::UnixShMem;
pub(crate) type MyShMemProvider = libafl_bolts::shmem::StdShMemProvider;
pub(crate) type MyMgr = libafl::events::llmp::restarting::LlmpRestartingEventManager<
    (),
    corpus::CallSequenceInput,
    MyState,
    MyShMem,
    MyShMemProvider,
>;

/// Result produced by a single worker.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkerResult {
    pub runs: u64,
    pub failures: Vec<PropertyFailure>,
}

/// A single property failure discovered during fuzzing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PropertyFailure {
    pub property_name: String,
    pub property_selector: [u8; 4],
    pub call_sequence: corpus::CallSequenceInput,
    /// Per-call block number / timestamp captured during execution.
    pub call_meta: Vec<crate::evm::CallMeta>,
}

/// Format a property failure's call sequence as a flat, Medusa-style log.
pub fn format_failure(artifact: &contract::ContractArtifact, failure: &PropertyFailure) -> String {
    let mut lines = Vec::new();
    for (i, call) in failure.call_sequence.calls.iter().enumerate() {
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
                        crate::evm::GAS_LIMIT,
                        crate::evm::CALLER,
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
                        crate::evm::GAS_LIMIT,
                        crate::evm::CALLER,
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
            crate::evm::GAS_LIMIT,
            crate::evm::CALLER,
            delay_suffix,
        ));
    }
    lines.join("\n")
}

/// Format a single decoded Solidity value for display.
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

pub struct Worker {
    artifact: contract::ContractArtifact,
    seeds: Vec<corpus::CallSequenceInput>,
    config: CampaignConfig,
    selectors: Vec<[u8; 4]>,
}

impl Worker {
    pub fn new(
        artifact: contract::ContractArtifact,
        seeds: Vec<corpus::CallSequenceInput>,
        config: CampaignConfig,
        selectors: Vec<[u8; 4]>,
    ) -> Self {
        Self {
            artifact,
            seeds,
            config,
            selectors,
        }
    }

    /// Run a LibAFL fuzzing loop for `max_runs` iterations.
    #[instrument(skip(self, state, mgr, coverage_map), fields(max_runs))]
    pub fn run(
        &self,
        state: Option<MyState>,
        mgr: &mut MyMgr,
        coverage_map: &mut [u8],
        max_runs: u64,
        client_id: usize,
    ) -> Result<WorkerResult> {
        info!(max_runs, "worker run starting");
        let map_ptr = coverage_map.as_mut_ptr();

        // checkrs: allow(unsafe_usage)
        let observer = unsafe {
            StdMapObserver::from_mut_ptr("edges", map_ptr, inspector::MAP_SIZE) //
        };
        let mut feedback = MaxMapFeedback::new(&observer);
        let mut objective = CrashFeedback::new();

        let mut state = match state {
            Some(s) => {
                trace!("restoring state from previous run");
                coverage_map.fill(0);
                s
            }
            None => {
                trace!("creating new state");
                let coverage_dir = self
                    .config
                    .corpus_dir
                    .as_ref()
                    .map(|d| d.join(format!("coverage/worker{client_id}")))
                    .unwrap_or_else(|| {
                        std::env::temp_dir().join(format!("raptor_coverage_{}", std::process::id()))
                    });
                let crash_dir = self
                    .config
                    .corpus_dir
                    .as_ref()
                    .map(|d| d.join(format!("crashes/worker{client_id}")))
                    .unwrap_or_else(|| {
                        std::env::temp_dir().join(format!("raptor_crashes_{}", std::process::id()))
                    });

                let corpus = MyCorpus::with_meta_format(
                    &coverage_dir,
                    Some(OnDiskMetadataFormat::JsonPretty),
                )
                .context("coverage corpus failed")?;
                let objectives = MyObjectiveCorpus::with_meta_format(
                    &crash_dir,
                    OnDiskMetadataFormat::JsonPretty,
                )
                .context("crash corpus failed")?;

                let mut s = StdState::new(
                    StdRand::with_seed(self.config.seed),
                    corpus,
                    objectives,
                    &mut feedback,
                    &mut objective,
                )?;
                let seeds = self.seeds.clone();
                for seed in seeds {
                    s.corpus_mut().add(Testcase::new(seed))?;
                }
                s
            }
        };

        let scheduler = QueueScheduler::new();
        let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

        let runner = evm::EvmRunner::from_target(&self.artifact)?;
        let mut failures = Vec::new();

        let mut harness = |input: &corpus::CallSequenceInput| {
            let inspector = inspector::CoverageInspector::from_slice(coverage_map);
            match runner.run_sequence(&input.calls, inspector) {
                Ok(res) if res.all_ok && res.property_triggered => {
                    trace!(property = %res.triggered_property.as_ref().unwrap(), "property triggered");
                    if let (Some(name), Some(sel)) =
                        (&res.triggered_property, &res.triggered_property_selector)
                    {
                        failures.push(PropertyFailure {
                            property_name: name.clone(),
                            property_selector: *sel,
                            call_sequence: input.clone(),
                            call_meta: res.call_meta.clone(),
                        });
                    }
                    libafl::executors::ExitKind::Crash
                }
                Ok(_) => libafl::executors::ExitKind::Ok,
                Err(_) => libafl::executors::ExitKind::Ok,
            }
        };

        let mut executor = InProcessExecutor::new(
            &mut harness,
            tuple_list!(observer),
            &mut fuzzer,
            &mut state,
            mgr,
        )?;
        debug!("executor created");

        let mut stages = tuple_list!(StdMutationalStage::with_max_iterations(
            libafl::mutators::scheduled::HavocScheduledMutator::new(tuple_list!(
                mutators::SequenceSwapMutator,
                mutators::SequenceInsertMutator::new(
                    self.selectors.clone(),
                    self.config.max_block_number_delay,
                    self.config.max_block_timestamp_delay,
                ),
                mutators::SequenceDeleteMutator,
                mutators::SequenceSpliceMutator,
                mutators::SequenceInterleaveMutator,
                mutators::SequenceHeadMutator,
                mutators::SequenceTailMutator,
                mutators::SequenceArgMutator::new(self.artifact.abi.clone()),
                mutators::SequenceDelayMutator::new(
                    self.config.max_block_number_delay,
                    self.config.max_block_timestamp_delay,
                ),
            )),
            std::num::NonZeroUsize::new(1).ok_or_else(|| libafl::Error::unknown("non-zero"))?,
        ));

        info!(max_runs, "starting fuzz loop");
        let mut runs = 0u64;
        for _ in 0..max_runs {
            fuzzer
                .fuzz_one(&mut stages, &mut executor, &mut state, mgr)
                .context("fuzz iteration failed")?;
            runs += 1;
        }
        info!(runs, "fuzz loop finished");

        Ok(WorkerResult { runs, failures })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::contract;
    use crate::corpus;
    use crate::evm;
    use crate::worker::PropertyFailure;
    use crate::worker::format_failure;

    #[test]
    fn format_failure_uses_block_number_and_timestamp_labels() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
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

        let failure = PropertyFailure {
            property_name: "property_caught".into(),
            property_selector: [0; 4],
            call_sequence: corpus::CallSequenceInput { calls },
            call_meta: vec![
                evm::CallMeta {
                    block_number: 0,
                    block_timestamp: 0,
                },
                evm::CallMeta {
                    block_number: 3,
                    block_timestamp: 4,
                },
                evm::CallMeta {
                    block_number: 4,
                    block_timestamp: 5,
                },
            ],
        };

        let output = format_failure(&artifact, &failure);
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
