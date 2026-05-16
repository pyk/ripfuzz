//! Per-process fuzzing worker that executes the LibAFL loop.

use std::fs;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use anyhow::{Context, Result};
use libafl::{
    corpus::{Corpus, InMemoryCorpus, Testcase},
    events::{EventConfig, SendExiting, launcher::Launcher},
    executors::InProcessExecutor,
    feedbacks::{CrashFeedback, MaxMapFeedback},
    fuzzer::{Fuzzer as LibAflFuzzer, StdFuzzer},
    observers::StdMapObserver,
    schedulers::QueueScheduler,
    stages::StdMutationalStage,
    state::{HasCorpus, StdState},
};
use libafl_bolts::{
    ownedref::OwnedMutSlice,
    rands::StdRand,
    shmem::{ShMem, ShMemProvider, StdShMemProvider},
    tuples::tuple_list,
};
use tracing::{debug, error, info, instrument, trace, warn};

use crate::campaign::{CampaignConfig, input};
use crate::contract;
use crate::evm;
use crate::inspector;

pub mod mutators;

type MyCorpus = InMemoryCorpus<input::CallSequenceInput>;
type MyState = StdState<MyCorpus, input::CallSequenceInput, StdRand, MyCorpus>;
type MyShMem = libafl_bolts::shmem::UnixShMem;
type MyShMemProvider = libafl_bolts::shmem::StdShMemProvider;
type MyMgr = libafl::events::llmp::restarting::LlmpRestartingEventManager<
    (),
    input::CallSequenceInput,
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
    pub call_sequence: input::CallSequenceInput,
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
    seeds: Vec<input::CallSequenceInput>,
    config: CampaignConfig,
    selectors: Vec<[u8; 4]>,
}

impl Worker {
    pub fn new(
        artifact: contract::ContractArtifact,
        seeds: Vec<input::CallSequenceInput>,
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
    ) -> Result<WorkerResult> {
        info!(max_runs, "worker run starting");
        let map_ptr = coverage_map.as_mut_ptr();

        let observer = StdMapObserver::from_mut_slice("edges", OwnedMutSlice::from(coverage_map));
        let mut feedback = MaxMapFeedback::new(&observer);
        let mut objective = CrashFeedback::new();

        let mut state = match state {
            Some(s) => {
                trace!("restoring state from previous run");
                unsafe { std::ptr::write_bytes(map_ptr, 0, inspector::MAP_SIZE) };
                s
            }
            None => {
                trace!("creating new state");
                let mut s = StdState::new(
                    StdRand::with_seed(self.config.seed),
                    InMemoryCorpus::<input::CallSequenceInput>::new(),
                    InMemoryCorpus::new(),
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

        let mut harness = |input: &input::CallSequenceInput| {
            let inspector =
                unsafe { inspector::CoverageInspector::new(map_ptr, inspector::MAP_SIZE) };
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

    /// Launch this worker via LibAFL's `Launcher` across `workers` cores.
    #[instrument(skip(self), fields(workers, max_runs = self.config.max_runs))]
    pub fn launch(&self, workers: usize, broker_port: u16) -> Result<WorkerResult> {
        info!(workers, "starting parallel fuzzing campaign");
        let mut shmem_provider = StdShMemProvider::new()?;
        let mut shmem = shmem_provider.new_shmem(inspector::MAP_SIZE)?;
        let map_ptr = shmem.as_mut_ptr();
        unsafe { std::ptr::write_bytes(map_ptr, 0, inspector::MAP_SIZE) };
        let map_desc = shmem.description();
        debug!("shared memory allocated for coverage map");

        let artifact = self.artifact.clone();
        let seeds = self.seeds.clone();
        let config = self.config.clone();
        let selectors = self.selectors.clone();

        let workers_u64 = workers as u64;
        let base_runs = config.max_runs / workers_u64;
        let remainder = (config.max_runs % workers_u64) as usize;
        debug!(base_runs, remainder, "run distribution calculated");

        // Unique identifier so we only collect temp files from this run.
        let campaign_id = format!(
            "{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let campaign_id_for_closure = campaign_id.clone();
        info!(%campaign_id, "campaign id generated");

        let monitor = libafl::monitors::SimpleMonitor::new(|s: &str| println!("{s}"));

        let run_client =
            move |state: Option<MyState>,
                  mut mgr: MyMgr,
                  _client: libafl::events::launcher::ClientDescription| {
                let client_id = _client.id();
                let core_id = _client.core_id().0;
                let pid = std::process::id();
                let local_max_runs = if core_id < remainder {
                    base_runs + 1
                } else {
                    base_runs
                };
                info!(client_id, core_id, pid, local_max_runs, "worker started");

                let mut local_provider = MyShMemProvider::new().map_err(|e| {
                    libafl::Error::illegal_state(format!("shmem provider failed: {e}"))
                })?;
                let mut local_shmem =
                    local_provider
                        .shmem_from_description(map_desc)
                        .map_err(|e| {
                            libafl::Error::illegal_state(format!("shmem mapping failed: {e}"))
                        })?;
                let map_slice = unsafe {
                    std::slice::from_raw_parts_mut(local_shmem.as_mut_ptr(), inspector::MAP_SIZE)
                };

                let worker = Worker::new(
                    artifact.clone(),
                    seeds.clone(),
                    config.clone(),
                    selectors.clone(),
                );
                let result = worker
                    .run(state, &mut mgr, map_slice, local_max_runs)
                    .map_err(|e| libafl::Error::illegal_state(format!("worker run failed: {e}")))?;

                // Persist local results so the campaign can aggregate them.
                let tmp = std::env::temp_dir()
                    .join(format!("raptor_{campaign_id_for_closure}_{pid}.json"));
                if let Ok(bytes) = serde_json::to_vec(&result) {
                    match fs::write(&tmp, &bytes) {
                        Err(e) => warn!(client_id, ?tmp, %e, "failed to write temp file"),
                        Ok(()) => debug!(client_id, ?tmp, "temp file written"),
                    }
                }

                // Tell LibAFL that this worker is done so the respawner
                // exits cleanly instead of panicking on a zero exit code.
                debug!(client_id, "calling send_exiting");
                mgr.send_exiting().map_err(|e| {
                    libafl::Error::illegal_state(format!("send_exiting failed: {e}"))
                })?;
                info!(client_id, "send_exiting succeeded, worker done");

                // Exit the child process immediately so it does not return
                // through Campaign and print duplicate campaign summaries.
                std::process::exit(0);
            };

        let cores = Self::workers_to_cores(workers)?;

        info!(workers, "spawning parallel fuzzers via Launcher");
        match Launcher::builder()
            .shmem_provider(shmem_provider)
            .monitor(monitor)
            .configuration(EventConfig::from_name("default"))
            .cores(&cores)
            .run_client(run_client)
            .stdout_file(Some("/dev/null"))
            .broker_port(broker_port)
            .build()
            .launch()
        {
            Ok(_) => {
                info!("Launcher exited normally");
            }
            Err(libafl::Error::ShuttingDown) => {
                info!("Launcher returned ShuttingDown (expected after send_exiting)");
            }
            Err(e) => {
                error!(%e, "Launcher failed unexpectedly");
                return Err(e).context("Parallel fuzzing failed");
            }
        }

        // Aggregate worker results from temp files.
        let mut total_runs = 0u64;
        let mut all_failures = Vec::new();
        let tmp_dir = std::env::temp_dir();
        let prefix = format!("raptor_{campaign_id}_");
        info!(%campaign_id, "aggregating temp files");

        let entries = match fs::read_dir(&tmp_dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(%e, ?tmp_dir, "failed to read temp dir");
                return Ok(WorkerResult {
                    runs: total_runs,
                    failures: all_failures,
                });
            }
        };
        let mut collected = 0usize;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(&prefix) || !name.ends_with(".json") {
                continue;
            }
            trace!(file = %name, "found matching temp file");
            let Ok(data) = fs::read(entry.path()) else {
                warn!(file = %name, "failed to read temp file");
                continue;
            };
            let Ok(result) = serde_json::from_slice::<WorkerResult>(&data) else {
                warn!(file = %name, "failed to parse temp file");
                continue;
            };
            total_runs += result.runs;
            all_failures.extend(result.failures);
            collected += 1;
        }
        info!(
            collected,
            total_runs,
            failures = all_failures.len(),
            "aggregation complete"
        );

        Ok(WorkerResult {
            runs: total_runs,
            failures: all_failures,
        })
    }

    /// Convert a worker count into a LibAFL `Cores` mask.
    fn workers_to_cores(workers: usize) -> Result<libafl_bolts::core_affinity::Cores> {
        let ids = libafl_bolts::core_affinity::get_core_ids()
            .map(|v| v.len())
            .unwrap_or(1);
        let count = workers.min(ids);

        let mask = if count >= ids {
            "all".into()
        } else {
            (0..count)
                .map(|i| format!("{i}"))
                .collect::<Vec<String>>()
                .join(",")
        };

        libafl_bolts::core_affinity::Cores::from_cmdline(&mask)
            .with_context(|| format!("failed to parse core mask '{mask}'"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::campaign::input;
    use crate::contract;
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
            input::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
            },
            input::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 3,
                block_timestamp_delay: 4,
            },
            input::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
            },
        ];

        let failure = PropertyFailure {
            property_name: "property_caught".into(),
            property_selector: [0; 4],
            call_sequence: input::CallSequenceInput { calls },
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
