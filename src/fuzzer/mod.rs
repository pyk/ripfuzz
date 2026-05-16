//! Coverage-guided fuzzing engine for Solidity smart contracts.

use std::cell::RefCell;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use anyhow::{Context, Result};
use libafl::{
    corpus::{Corpus, InMemoryCorpus, Testcase},
    events::{EventConfig, NopEventManager, launcher::Launcher},
    executors::InProcessExecutor,
    feedbacks::{CrashFeedback, MaxMapFeedback},
    fuzzer::{Fuzzer as LibAflFuzzer, StdFuzzer},
    monitors::SimpleMonitor,
    mutators::scheduled::HavocScheduledMutator,
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

use crate::contract;
use crate::evm;
use crate::inspector;

pub mod config;
pub mod mutators;
pub mod sequence;

/// A single property failure discovered during fuzzing.
#[derive(Debug, Clone)]
pub struct PropertyFailure {
    pub property_name: String,
    pub property_selector: [u8; 4],
    pub call_sequence: sequence::CallSequenceInput,
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
    artifact: contract::ContractArtifact,
    seeds: Vec<sequence::CallSequenceInput>,
    config: config::FuzzConfig,
    selectors: Vec<[u8; 4]>,
}

impl Fuzzer {
    /// Deploy the artifact, generate seeds, and prepare the fuzzer with default config.
    pub fn from_artifact(artifact: contract::ContractArtifact) -> Result<Self> {
        Self::from_artifact_with_config(artifact, config::FuzzConfig::default())
    }

    /// Deploy the artifact, generate seeds, and prepare the fuzzer.
    pub fn from_artifact_with_config(
        artifact: contract::ContractArtifact,
        config: config::FuzzConfig,
    ) -> Result<Self> {
        let _runner = evm::EvmRunner::from_target(&artifact)?;
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
    pub fn launch(&self, cores: &libafl_bolts::core_affinity::Cores) -> anyhow::Result<()> {
        let mut shmem_provider = StdShMemProvider::new()?;
        let mut shmem = shmem_provider.new_shmem(inspector::MAP_SIZE)?;
        let map_ptr = shmem.as_mut_ptr();
        unsafe { std::ptr::write_bytes(map_ptr, 0, inspector::MAP_SIZE) };
        let map_desc = shmem.description();

        let artifact = self.artifact.clone();
        let seeds = self.seeds.clone();
        let config = self.config.clone();
        let selectors = self.selectors.clone();

        let monitor = SimpleMonitor::new(|s: &str| println!("{s}"));

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

                let mut harness = |input: &sequence::CallSequenceInput| {
                    let inspector = unsafe {
                        inspector::CoverageInspector::new(map_slice_ptr, inspector::MAP_SIZE)
                    };
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
            .context("Parallel fuzzing failed")?;

        Ok(())
    }

    /// Format a property failure's call sequence as a flat, Medusa-style log.
    pub fn format_failure(&self, failure: &PropertyFailure) -> String {
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
                    };

                    let tuple = DynSolType::Tuple(types);
                    let Ok(decoded) = tuple.abi_decode_params(&call.args) else {
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

    fn run_with_config(&self, config: &config::FuzzConfig) -> Result<FuzzResult> {
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
                mutators::SequenceSwapMutator,
                mutators::SequenceInsertMutator::new(
                    self.selectors.clone(),
                    config.max_block_number_delay,
                    config.max_block_timestamp_delay,
                ),
                mutators::SequenceDeleteMutator,
                mutators::SequenceSpliceMutator,
                mutators::SequenceInterleaveMutator,
                mutators::SequenceHeadMutator,
                mutators::SequenceTailMutator,
                mutators::SequenceArgMutator::new(self.artifact.abi.clone()),
                mutators::SequenceDelayMutator::new(
                    config.max_block_number_delay,
                    config.max_block_timestamp_delay,
                ),
            )),
            std::num::NonZeroUsize::new(1).ok_or_else(|| libafl::Error::unknown("non-zero"))?,
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
    match v {
        DynSolValue::Bool(b) => format!("{}", b),
        DynSolValue::Int(i, _) => format!("{}", i),
        DynSolValue::Uint(u, _) => format!("{}", u),
        DynSolValue::Address(a) => format!("{:?}", a),
        DynSolValue::String(s) => format!("\"{}\"", s),
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b)),
        _ => format!("{:?}", v),
    }
}

/// Build seed inputs from the contract ABI.
fn build_seeds(
    artifact: &contract::ContractArtifact,
    max_len: usize,
) -> Vec<sequence::CallSequenceInput> {
    let mut seeds = Vec::new();

    // Single-call seeds for every ABI function.
    for func in artifact.abi.functions() {
        let selector: [u8; 4] = func.selector().into();
        let call = sequence::Call {
            selector,
            args: vec![0u8; func.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        };
        seeds.push(sequence::CallSequenceInput::single(call));
    }

    // Combined seed with all non-view/pure action functions in ABI order.
    let action_calls: Vec<sequence::Call> = artifact
        .abi
        .functions()
        .filter(|f| {
            !matches!(
                f.state_mutability,
                alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
            )
        })
        .map(|f| sequence::Call {
            selector: f.selector().into(),
            args: vec![0u8; f.inputs.len() * 32],
            block_number_delay: 0,
            block_timestamp_delay: 0,
        })
        .collect();

    if !action_calls.is_empty() {
        let mut combined = sequence::CallSequenceInput::new();
        combined.calls = action_calls.clone();
        seeds.push(combined);
    }

    // Permutation seeds for action functions (up to max_len).
    let n = action_calls.len();
    if n > 0 && n <= max_len {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(Vec::new());
        let mut permutations = Vec::new();
        while let Some(prefix) = queue.pop_front() {
            if prefix.len() == n {
                permutations.push(prefix);
                continue;
            }
            for (idx, _call) in action_calls.iter().enumerate() {
                let already_in_prefix = prefix.contains(&idx);
                if !already_in_prefix {
                    let mut next = prefix.to_vec();
                    next.push(idx);
                    queue.push_back(next);
                }
            }
        }
        for perm in permutations {
            let mut seq = sequence::CallSequenceInput::new();
            for &i in &perm {
                seq.calls.push(action_calls[i].replicate());
            }
            seeds.push(seq);
        }
    }

    seeds
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use crate::contract;
    use crate::evm;
    use crate::fuzzer::Fuzzer;
    use crate::fuzzer::PropertyFailure;
    use crate::fuzzer::sequence;

    #[test]
    fn deployment_reports_constructor_revert_reason() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ConstructorRevert.sol"),
        )
        .unwrap();

        let err = Fuzzer::from_artifact(artifact).unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ConstructorRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn deployment_reports_complex_constructor_trace() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/ComplexConstructorRevert.sol"),
        )
        .unwrap();

        let err = Fuzzer::from_artifact(artifact).unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/ComplexConstructorRevertOutput.txt")
                .unwrap();
        assert_eq!(msg, expected);
    }
    #[test]
    fn deployment_reports_set_up_revert_trace() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/SetupRevert.sol"),
        )
        .unwrap();

        let err = Fuzzer::from_artifact(artifact).unwrap_err();
        let msg = format!("{err}");
        let expected =
            fs::read_to_string("fixtures/basic-target/test/SetupRevertOutput.txt").unwrap();
        assert_eq!(msg, expected);
    }

    #[test]
    fn catches_l1_simple_knob_dragon() {
        let artifact = contract::ContractBuilder::build(
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
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/challenges"),
            Path::new("src/L1SimpleKnob.sol"),
        )
        .unwrap();

        let fuzzer = Fuzzer::from_artifact(artifact).unwrap();

        let calls = vec![
            sequence::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
            },
            sequence::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 3,
                block_timestamp_delay: 4,
            },
            sequence::Call {
                selector: [0x0a, 0x92, 0x54, 0xe4],
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
            },
        ];

        let failure = PropertyFailure {
            property_name: "property_caught".into(),
            property_selector: [0; 4],
            call_sequence: sequence::CallSequenceInput { calls },
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
