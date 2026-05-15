use std::cell::RefCell;

use anyhow::Result;
use libafl::{
    corpus::{Corpus, InMemoryCorpus, Testcase},
    events::NopEventManager,
    executors::InProcessExecutor,
    feedbacks::{CrashFeedback, MaxMapFeedback},
    fuzzer::{Fuzzer as LibAflFuzzer, StdFuzzer},
    mutators::scheduled::HavocScheduledMutator,
    observers::StdMapObserver,
    schedulers::QueueScheduler,
    stages::StdMutationalStage,
    state::{HasCorpus, StdState},
};
use libafl_bolts::{ownedref::OwnedMutSlice, rands::StdRand, tuples::tuple_list};

use crate::contract::ContractArtifact;
use crate::evm::EvmRunner;
use crate::fuzzer::config::FuzzConfig;
use crate::fuzzer::mutators::{
    SequenceArgMutator, SequenceDelayMutator, SequenceDeleteMutator, SequenceHeadMutator,
    SequenceInsertMutator, SequenceInterleaveMutator, SequenceSpliceMutator, SequenceSwapMutator,
    SequenceTailMutator,
};
use crate::fuzzer::sequence::{Call, CallSequenceInput};

pub mod config;
pub mod mutators;
pub mod sequence;

/// A single property failure discovered during fuzzing.
#[derive(Debug, Clone)]
pub struct PropertyFailure {
    pub property_name: String,
    pub property_selector: [u8; 4],
    pub call_sequence: CallSequenceInput,
    /// Per-call block number / timestamp captured during execution.
    pub call_meta: Vec<crate::evm::CallMeta>,
}

/// The result of a fuzzing campaign.
#[derive(Debug)]
pub struct FuzzResult {
    pub iterations: u64,
    pub failures: Vec<PropertyFailure>,
}

/// A fuzzer configured to run against a deployed contract.
#[derive(Debug)]
pub struct Fuzzer {
    artifact: ContractArtifact,
    seeds: Vec<CallSequenceInput>,
    config: FuzzConfig,
    selectors: Vec<[u8; 4]>,
}

impl Fuzzer {
    /// Deploy the artifact, generate seeds, and prepare the fuzzer with default config.
    pub fn from_artifact(artifact: ContractArtifact) -> Result<Self> {
        Self::from_artifact_with_config(artifact, FuzzConfig::default())
    }

    /// Deploy the artifact, generate seeds, and prepare the fuzzer.
    pub fn from_artifact_with_config(
        artifact: ContractArtifact,
        config: FuzzConfig,
    ) -> Result<Self> {
        let _runner = EvmRunner::from_target(&artifact)?;
        let seeds = build_seeds(&artifact, config.sequence_length);
        let selectors: Vec<[u8; 4]> = artifact
            .abi
            .functions()
            .map(|f| f.selector().into())
            .collect();
        Ok(Self {
            artifact,
            seeds,
            config,
            selectors,
        })
    }

    /// Run the fuzz loop and return a summary of the results.
    pub fn run(&self) -> Result<FuzzResult> {
        self.run_with_config(&self.config.clone())
    }

    /// Run the fuzz loop with a specific configuration.
    pub fn run_with_iters(&self, max_iters: u64) -> Result<FuzzResult> {
        let mut config = self.config.clone();
        config.max_iters = max_iters;
        self.run_with_config(&config)
    }

    /// Launch a parallel fuzzing campaign across the given cores.
    pub fn launch(
        &self,
        cores: &libafl_bolts::core_affinity::Cores,
    ) -> anyhow::Result<()> {
        use libafl::{
            corpus::{Corpus, Testcase},
            events::{EventConfig, launcher::Launcher},
            monitors::SimpleMonitor,
            observers::StdMapObserver,
            stages::StdMutationalStage,
            state::HasCorpus,
        };
        use libafl_bolts::{
            ownedref::OwnedMutSlice,
            rands::StdRand,
            shmem::{ShMem, ShMemProvider, StdShMemProvider},
            tuples::tuple_list,
        };
        use crate::inspector::{CoverageInspector, MAP_SIZE};

        let mut shmem_provider = StdShMemProvider::new()?;
        let mut shmem = shmem_provider.new_shmem(MAP_SIZE)?;
        let map_ptr = shmem.as_mut_ptr();
        unsafe { std::ptr::write_bytes(map_ptr, 0, MAP_SIZE) };
        let map_desc = shmem.description();

        let artifact = self.artifact.clone();
        let seeds = self.seeds.clone();
        let config = self.config.clone();
        let selectors = self.selectors.clone();

        let monitor = SimpleMonitor::new(|s: &str| println!("{s}"));

        type MyCorpus = libafl::corpus::InMemoryCorpus<CallSequenceInput>;
        type MyState = libafl::state::StdState<MyCorpus, CallSequenceInput, StdRand, MyCorpus>;
        type MyShMem = libafl_bolts::shmem::UnixShMem;
        type MyShMemProvider = libafl_bolts::shmem::StdShMemProvider;
        type MyMgr = libafl::events::llmp::restarting::LlmpRestartingEventManager<
            (),
            CallSequenceInput,
            MyState,
            MyShMem,
            MyShMemProvider,
        >;

        let run_client = move |state: Option<MyState>, mut mgr: MyMgr, _client: libafl::events::launcher::ClientDescription| {
            let mut local_provider = MyShMemProvider::new().map_err(|e| {
                libafl::Error::illegal_state(format!("shmem provider failed: {e}"))
            })?;
            let mut local_shmem = local_provider
                .shmem_from_description(map_desc)
                .map_err(|e| {
                    libafl::Error::illegal_state(format!("shmem mapping failed: {e}"))
                })?;
            let map_slice = unsafe {
                std::slice::from_raw_parts_mut(local_shmem.as_mut_ptr(), MAP_SIZE)
            };
            let map_slice_ptr = map_slice.as_mut_ptr();

            let runner = EvmRunner::from_target(&artifact).map_err(|e| {
                libafl::Error::illegal_state(format!("EVM runner creation failed: {e}"))
            })?;

            let observer = StdMapObserver::from_mut_slice("edges", OwnedMutSlice::from(map_slice));
            let mut feedback = libafl::feedbacks::MaxMapFeedback::new(&observer);
            let mut objective = libafl::feedbacks::CrashFeedback::new();

            let mut state = match state {
                Some(s) => {
                    unsafe { std::ptr::write_bytes(map_slice_ptr, 0, MAP_SIZE) };
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
                    for seed in &seeds {
                        s.corpus_mut()
                            .add(Testcase::new(seed.clone()))
                            .map_err(|e| {
                                libafl::Error::illegal_state(format!(
                                    "Seed addition failed: {e}"
                                ))
                            })?;
                    }
                    s
                }
            };

            let scheduler = libafl::schedulers::QueueScheduler::new();
            let mut fuzzer = libafl::fuzzer::StdFuzzer::new(scheduler, feedback, objective);

            let mut harness = |input: &CallSequenceInput| {
                let inspector = unsafe { CoverageInspector::new(map_slice_ptr, MAP_SIZE) };
                match runner.run_sequence(&input.calls, inspector) {
                    Ok(res) if res.all_ok && res.property_triggered => {
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

            let mut stages = tuple_list!(StdMutationalStage::with_max_iterations(
                libafl::mutators::scheduled::HavocScheduledMutator::new(tuple_list!(
                    SequenceSwapMutator,
                    SequenceInsertMutator::new(
                        selectors.clone(),
                        config.max_block_number_delay,
                        config.max_block_timestamp_delay,
                    ),
                    SequenceDeleteMutator,
                    SequenceSpliceMutator,
                    SequenceInterleaveMutator,
                    SequenceHeadMutator,
                    SequenceTailMutator,
                    SequenceArgMutator::new(artifact.abi.clone()),
                    SequenceDelayMutator::new(
                        config.max_block_number_delay,
                        config.max_block_timestamp_delay,
                    ),
                )),
                std::num::NonZeroUsize::new(1).unwrap(),
            ));

            for _ in 0..config.max_iters {
                fuzzer
                    .fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr)
                    .map_err(|e| {
                        libafl::Error::illegal_state(format!("Fuzz iteration failed: {e}"))
                    })?;
            }

            Ok(())
        };

        Launcher::builder()
            .shmem_provider(shmem_provider)
            .monitor(monitor)
            .configuration(EventConfig::from_name("default"))
            .cores(cores)
            .run_client(run_client)
            .build()
            .launch()
            .map_err(|e| anyhow::anyhow!("Parallel fuzzing failed: {e}"))?;

        Ok(())
    }

    /// Format a property failure's call sequence as a flat, Medusa-style log.
    pub fn format_failure(&self, failure: &PropertyFailure) -> String {
        use alloy_dyn_abi::{DynSolType, DynSolValue};

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

            let func = self
                .artifact
                .abi
                .functions()
                .find(|f| f.selector().as_slice() == call.selector);

            let func_name = func
                .map(|f| f.name.clone())
                .unwrap_or_else(|| format!("0x{}", hex::encode(call.selector)));

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
                    "()".to_string()
                } else {
                    let types: Vec<DynSolType> = match func_abi
                        .inputs
                        .iter()
                        .map(|p| p.selector_type().parse::<DynSolType>())
                        .collect::<Result<_, _>>()
                    {
                        Ok(t) => t,
                        Err(_) => {
                            let raw = format!("(0x{})", hex::encode(&call.args));
                            lines.push(format!(
                                "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
                                n,
                                self.artifact.contract_name,
                                func_name,
                                raw,
                                block,
                                time,
                                crate::evm::GAS_LIMIT,
                                crate::evm::CALLER,
                                delay_suffix,
                            ));
                            continue;
                        }
                    };

                    let tuple = DynSolType::Tuple(types);
                    let decoded = match tuple.abi_decode_params(&call.args) {
                        Ok(d) => d,
                        Err(_) => {
                            let raw = format!("(0x{})", hex::encode(&call.args));
                            lines.push(format!(
                                "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
                                n,
                                self.artifact.contract_name,
                                func_name,
                                raw,
                                block,
                                time,
                                crate::evm::GAS_LIMIT,
                                crate::evm::CALLER,
                                delay_suffix,
                            ));
                            continue;
                        }
                    };

                    let values = match decoded {
                        DynSolValue::Tuple(v) => v,
                        other => vec![other],
                    };

                    let args_str = values
                        .iter()
                        .map(format_dyn_value)
                        .collect::<Vec<_>>()
                        .join(", ");

                    format!("({})", args_str)
                }
            } else {
                format!("0x{}", hex::encode(&call.args))
            };

            lines.push(format!(
                "{}) {}::{}{} (block_number={}, block_timestamp={}, gas={}, gasprice=1, value=0, sender={:?}{})",
                n,
                self.artifact.contract_name,
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

    fn run_with_config(&self, config: &FuzzConfig) -> Result<FuzzResult> {
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
            StdRand::with_seed(config.seed),
            InMemoryCorpus::<CallSequenceInput>::new(),
            InMemoryCorpus::new(),
            &mut feedback,
            &mut objective,
        )?;

        for seed in &self.seeds {
            state.corpus_mut().add(Testcase::new(seed.clone()))?;
        }

        let scheduler = QueueScheduler::new();
        let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

        let runner = EvmRunner::from_target(&self.artifact)?;
        let failures = RefCell::new(Vec::new());

        let mut harness = |input: &CallSequenceInput| {
            let inspector = crate::inspector::CoverageInspector::global();
            match runner.run_sequence(&input.calls, inspector) {
            Ok(res) if res.all_ok && res.property_triggered => {
                if let (Some(name), Some(sel)) =
                    (&res.triggered_property, &res.triggered_property_selector)
                {
                    failures.borrow_mut().push(PropertyFailure {
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
            HavocScheduledMutator::new(tuple_list!(
                SequenceSwapMutator,
                SequenceInsertMutator::new(
                    self.selectors.clone(),
                    config.max_block_number_delay,
                    config.max_block_timestamp_delay,
                ),
                SequenceDeleteMutator,
                SequenceSpliceMutator,
                SequenceInterleaveMutator,
                SequenceHeadMutator,
                SequenceTailMutator,
                SequenceArgMutator::new(self.artifact.abi.clone()),
                SequenceDelayMutator::new(
                    config.max_block_number_delay,
                    config.max_block_timestamp_delay,
                ),
            )),
            std::num::NonZeroUsize::new(1).unwrap(),
        ));

        let mut manager = NopEventManager::new();
        let mut iterations = 0;
        for _ in 0..config.max_iters {
            if !failures.borrow().is_empty() {
                break;
            }
            fuzzer.fuzz_one(&mut stages, &mut executor, &mut state, &mut manager)?;
            iterations += 1;
        }

        Ok(FuzzResult {
            iterations,
            failures: failures.into_inner(),
        })
    }
}

/// Format a single decoded Solidity value for display.
fn format_dyn_value(v: &alloy_dyn_abi::DynSolValue) -> String {
    use alloy_dyn_abi::DynSolValue;
    match v {
        DynSolValue::Bool(b) => b.to_string(),
        DynSolValue::Int(i, _) => i.to_string(),
        DynSolValue::Uint(u, _) => u.to_string(),
        DynSolValue::Address(a) => format!("{:?}", a),
        DynSolValue::String(s) => format!("\"{}\"", s),
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        _ => format!("{:?}", v),
    }
}

/// Build seed inputs from the contract ABI.
fn build_seeds(artifact: &ContractArtifact, max_len: usize) -> Vec<CallSequenceInput> {
    let mut seeds = Vec::new();

    // Single-call seeds for every ABI function.
    for func in artifact.abi.functions() {
        let selector: [u8; 4] = func.selector().into();
        let call = Call {
            selector,
            args: vec![0u8; func.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        };
        seeds.push(CallSequenceInput::single(call));
    }

    // Combined seed with all non-view/pure action functions in ABI order.
    let action_calls: Vec<Call> = artifact
        .abi
        .functions()
        .filter(|f| {
            !matches!(
                f.state_mutability,
                alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
            )
        })
        .map(|f| Call {
            selector: f.selector().into(),
            args: vec![0u8; f.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        })
        .collect();

    if !action_calls.is_empty() {
        let mut combined = CallSequenceInput::new();
        for call in &action_calls {
            combined.calls.push(call.clone());
        }
        seeds.push(combined);
    }

    // Permutation seeds for action functions (up to max_len).
    let n = action_calls.len();
    if n > 0 && n <= max_len {
        use std::collections::VecDeque;
        let mut queue = VecDeque::new();
        queue.push_back(Vec::new());
        let mut permutations = Vec::new();
        while let Some(prefix) = queue.pop_front() {
            if prefix.len() == n {
                permutations.push(prefix);
                continue;
            }
            for call in &action_calls {
                if !prefix.contains(call) {
                    let mut next = prefix.clone();
                    next.push(call.clone());
                    queue.push_back(next);
                }
            }
        }
        for perm in permutations {
            let mut seq = CallSequenceInput::new();
            seq.calls = perm;
            seeds.push(seq);
        }
    }

    seeds
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::contract::ContractBuilder;
    use crate::fuzzer::Fuzzer;

    #[test]
    fn deployment_reports_constructor_revert_reason() {
        let artifact = ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ConstructorRevert.sol"),
        )
        .unwrap();

        let err = Fuzzer::from_artifact(artifact).unwrap_err();
        let msg = format!("{err}");
        let expected =
            std::fs::read_to_string("fixtures/basic-target/test/ConstructorRevertOutput.txt")
                .unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_complex_constructor_trace() {
        let artifact = ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ComplexConstructorRevert.sol"),
        )
        .unwrap();

        let err = Fuzzer::from_artifact(artifact).unwrap_err();
        let msg = format!("{err}");
        let expected = std::fs::read_to_string(
            "fixtures/basic-target/test/ComplexConstructorRevertOutput.txt",
        )
        .unwrap();
        assert_eq!(msg, expected);
    }
    #[test]
    fn deployment_reports_set_up_revert_trace() {
        let artifact = ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/SetupRevert.sol"),
        )
        .unwrap();

        let err = Fuzzer::from_artifact(artifact).unwrap_err();
        let msg = format!("{err}");
        let expected =
            std::fs::read_to_string("fixtures/basic-target/test/SetupRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn catches_l1_simple_knob_dragon() {
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let artifact = ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
        .unwrap();

        assert!(
            !artifact.properties.is_empty(),
            "property_caught() should be discovered as a property"
        );

        let fuzzer = Fuzzer::from_artifact(artifact).unwrap();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(fuzzer.run_with_iters(10_000));
        });

        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("fuzzing should complete within 5 seconds")
            .expect("fuzz run should succeed");

        assert!(
            !result.failures.is_empty(),
            "raptor should find at least one property failure (dragon caught)"
        );
    }

    #[test]
    fn format_failure_uses_block_number_and_timestamp_labels() {
        use crate::contract::ContractBuilder;
        use crate::evm::CallMeta;
        use crate::fuzzer::PropertyFailure;
        use crate::fuzzer::sequence::{Call, CallSequenceInput};
        use std::path::Path;

        let artifact = ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
        .unwrap();

        let fuzzer = Fuzzer::from_artifact(artifact).unwrap();

        let calls = vec![
            Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
            },
            Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 3,
                block_timestamp_delay: 4,
            },
            Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
            },
        ];

        let failure = PropertyFailure {
            property_name: "property_caught".to_string(),
            property_selector: [0; 4],
            call_sequence: CallSequenceInput { calls },
            call_meta: vec![
                CallMeta {
                    block_number: 0,
                    block_timestamp: 0,
                },
                CallMeta {
                    block_number: 3,
                    block_timestamp: 4,
                },
                CallMeta {
                    block_number: 4,
                    block_timestamp: 5,
                },
            ],
        };

        let output = fuzzer.format_failure(&failure);
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
