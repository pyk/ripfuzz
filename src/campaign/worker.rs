//! Per-process fuzzing worker that executes the LibAFL loop.

use std::cell::RefCell;
use std::fs;

use anyhow::{Context, Result};
use libafl::{
    corpus::{Corpus, InMemoryCorpus, Testcase},
    events::{EventConfig, NopEventManager, launcher::Launcher},
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

use crate::campaign::CampaignConfig;
use crate::contract;
use crate::evm;
use crate::fuzzer;
use crate::fuzzer::mutators;
use crate::fuzzer::sequence;
use crate::inspector;

/// Result produced by a single worker.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WorkerResult {
    pub iterations: u64,
    pub failures: Vec<fuzzer::PropertyFailure>,
}

/// A worker configured to run against a deployed contract.
pub struct Worker {
    artifact: contract::ContractArtifact,
    seeds: Vec<sequence::CallSequenceInput>,
    config: CampaignConfig,
    selectors: Vec<[u8; 4]>,
}

impl Worker {
    pub fn new(
        artifact: contract::ContractArtifact,
        seeds: Vec<sequence::CallSequenceInput>,
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

    /// Run a single-threaded fuzzing loop and return the local result.
    pub fn run_single(&self) -> Result<WorkerResult> {
        let map = unsafe {
            std::slice::from_raw_parts_mut(
                std::ptr::addr_of_mut!(crate::inspector::COVERAGE_MAP).cast::<u8>(),
                crate::inspector::MAP_SIZE,
            )
        };
        let observer = StdMapObserver::from_mut_slice("edges", OwnedMutSlice::from(map));
        let mut feedback = MaxMapFeedback::new(&observer);
        let mut objective = CrashFeedback::new();

        let mut state = StdState::new(
            StdRand::with_seed(self.config.seed),
            InMemoryCorpus::<sequence::CallSequenceInput>::new(),
            InMemoryCorpus::new(),
            &mut feedback,
            &mut objective,
        )?;

        let seeds = self.seeds.clone();
        for seed in seeds {
            state.corpus_mut().add(Testcase::new(seed))?;
        }

        let scheduler = QueueScheduler::new();
        let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

        let runner = evm::EvmRunner::from_target(&self.artifact)?;
        let failures = RefCell::new(Vec::new());

        let mut harness = |input: &sequence::CallSequenceInput| {
            let inspector = crate::inspector::CoverageInspector::global();
            match runner.run_sequence(&input.calls, inspector) {
                Ok(res) if res.all_ok && res.property_triggered => {
                    if let (Some(name), Some(sel)) =
                        (&res.triggered_property, &res.triggered_property_selector)
                    {
                        failures.borrow_mut().push(fuzzer::PropertyFailure {
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
            &mut NopEventManager::new(),
        )?;

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

        let mut manager = NopEventManager::new();
        let mut iterations = 0;
        for _ in 0..self.config.max_iters {
            if !failures.borrow().is_empty() {
                break;
            }
            fuzzer.fuzz_one(&mut stages, &mut executor, &mut state, &mut manager)?;
            iterations += 1;
        }

        Ok(WorkerResult {
            iterations,
            failures: failures.into_inner(),
        })
    }

    /// Launch a parallel fuzzing campaign across the given number of workers.
    pub fn run_parallel(&self, workers: usize) -> Result<WorkerResult> {
        let mut shmem_provider = StdShMemProvider::new()?;
        let mut shmem = shmem_provider.new_shmem(inspector::MAP_SIZE)?;
        let map_ptr = shmem.as_mut_ptr();
        unsafe { std::ptr::write_bytes(map_ptr, 0, inspector::MAP_SIZE) };
        let map_desc = shmem.description();

        let artifact = self.artifact.clone();
        let seeds = self.seeds.clone();
        let config = self.config.clone();
        let selectors = self.selectors.clone();

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

        let monitor = libafl::monitors::SimpleMonitor::new(|s: &str| println!("{s}"));

        type MyCorpus = libafl::corpus::InMemoryCorpus<sequence::CallSequenceInput>;
        type MyState =
            libafl::state::StdState<MyCorpus, sequence::CallSequenceInput, StdRand, MyCorpus>;
        type MyShMem = libafl_bolts::shmem::UnixShMem;
        type MyShMemProvider = libafl_bolts::shmem::StdShMemProvider;
        type MyMgr = libafl::events::llmp::restarting::LlmpRestartingEventManager<
            (),
            sequence::CallSequenceInput,
            MyState,
            MyShMem,
            MyShMemProvider,
        >;

        let run_client =
            move |state: Option<MyState>,
                  mut mgr: MyMgr,
                  _client: libafl::events::launcher::ClientDescription| {
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
                let map_slice_ptr = map_slice.as_mut_ptr();

                let runner = evm::EvmRunner::from_target(&artifact).map_err(|e| {
                    libafl::Error::illegal_state(format!("EVM runner creation failed: {e}"))
                })?;

                let observer =
                    StdMapObserver::from_mut_slice("edges", OwnedMutSlice::from(map_slice));
                let mut feedback = libafl::feedbacks::MaxMapFeedback::new(&observer);
                let mut objective = libafl::feedbacks::CrashFeedback::new();

                let mut state = match state {
                    Some(s) => {
                        unsafe { std::ptr::write_bytes(map_slice_ptr, 0, inspector::MAP_SIZE) };
                        s
                    }
                    None => {
                        let mut s = MyState::new(
                            StdRand::with_seed(config.seed),
                            MyCorpus::new(),
                            MyCorpus::new(),
                            &mut feedback,
                            &mut objective,
                        )
                        .map_err(|e| {
                            libafl::Error::illegal_state(format!("State creation failed: {e}"))
                        })?;
                        for seed in seeds {
                            s.corpus_mut().add(Testcase::new(seed)).map_err(|e| {
                                libafl::Error::illegal_state(format!("Seed addition failed: {e}"))
                            })?;
                        }
                        s
                    }
                };

                let scheduler = libafl::schedulers::QueueScheduler::new();
                let mut fuzzer = libafl::fuzzer::StdFuzzer::new(scheduler, feedback, objective);

                let mut local_failures = Vec::new();
                let mut iterations = 0u64;

                let mut harness = |input: &sequence::CallSequenceInput| {
                    let inspector = unsafe {
                        inspector::CoverageInspector::new(map_slice_ptr, inspector::MAP_SIZE)
                    };
                    match runner.run_sequence(&input.calls, inspector) {
                        Ok(res) if res.all_ok && res.property_triggered => {
                            if let (Some(name), Some(sel)) =
                                (&res.triggered_property, &res.triggered_property_selector)
                            {
                                local_failures.push(fuzzer::PropertyFailure {
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

                let mut executor = libafl::executors::InProcessExecutor::new(
                    &mut harness,
                    tuple_list!(observer),
                    &mut fuzzer,
                    &mut state,
                    &mut mgr,
                )
                .map_err(|e| {
                    libafl::Error::illegal_state(format!("Executor creation failed: {e}"))
                })?;

                let mut stages =
                    tuple_list!(libafl::stages::StdMutationalStage::with_max_iterations(
                        libafl::mutators::scheduled::HavocScheduledMutator::new(tuple_list!(
                            mutators::SequenceSwapMutator,
                            mutators::SequenceInsertMutator::new(
                                selectors.clone(),
                                config.max_block_number_delay,
                                config.max_block_timestamp_delay,
                            ),
                            mutators::SequenceDeleteMutator,
                            mutators::SequenceSpliceMutator,
                            mutators::SequenceInterleaveMutator,
                            mutators::SequenceHeadMutator,
                            mutators::SequenceTailMutator,
                            mutators::SequenceArgMutator::new(artifact.abi.clone()),
                            mutators::SequenceDelayMutator::new(
                                config.max_block_number_delay,
                                config.max_block_timestamp_delay,
                            ),
                        )),
                        std::num::NonZeroUsize::new(1)
                            .ok_or_else(|| libafl::Error::unknown("non-zero"))?,
                    ));

                for _ in 0..config.max_iters {
                    fuzzer
                        .fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr)
                        .map_err(|e| {
                            libafl::Error::illegal_state(format!("Fuzz iteration failed: {e}"))
                        })?;
                    iterations += 1;
                }

                // Persist local results so the campaign can aggregate them.
                let result = WorkerResult {
                    iterations,
                    failures: local_failures,
                };
                let pid = std::process::id();
                let tmp = std::env::temp_dir()
                    .join(format!("raptor_{campaign_id_for_closure}_{pid}.json"));
                if let Ok(bytes) = serde_json::to_vec(&result) {
                    let _ = fs::write(&tmp, bytes);
                }

                Ok(())
            };

        let cores = Self::workers_to_cores(workers)?;

        Launcher::builder()
            .shmem_provider(shmem_provider)
            .monitor(monitor)
            .configuration(EventConfig::from_name("default"))
            .cores(&cores)
            .run_client(run_client)
            .build()
            .launch()
            .context("Parallel fuzzing failed")?;

        // Aggregate worker results from temp files.
        let mut total_iterations = 0u64;
        let mut all_failures = Vec::new();
        let tmp_dir = std::env::temp_dir();
        let prefix = format!("raptor_{campaign_id}_");

        let entries = match fs::read_dir(&tmp_dir) {
            Ok(e) => e,
            Err(_) => {
                return Ok(WorkerResult {
                    iterations: total_iterations,
                    failures: all_failures,
                });
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(&prefix) || !name.ends_with(".json") {
                continue;
            }
            let Ok(data) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(result) = serde_json::from_slice::<WorkerResult>(&data) else {
                continue;
            };
            total_iterations += result.iterations;
            all_failures.extend(result.failures);
            let _ = fs::remove_file(entry.path());
        }

        Ok(WorkerResult {
            iterations: total_iterations,
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
