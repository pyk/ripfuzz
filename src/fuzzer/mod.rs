use std::cell::RefCell;

use anyhow::Result;
use libafl::{
    corpus::{Corpus, InMemoryCorpus, Testcase},
    events::NopEventManager,
    executors::InProcessExecutor,
    feedbacks::{CrashFeedback, MaxMapFeedback},
    fuzzer::{Fuzzer as LibAflFuzzer, StdFuzzer},
    inputs::HasTargetBytes,
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
    SequenceArgMutator, SequenceDeleteMutator, SequenceHeadMutator, SequenceInsertMutator,
    SequenceInterleaveMutator, SequenceSpliceMutator, SequenceSwapMutator, SequenceTailMutator,
};
use crate::fuzzer::sequence::{Call, CallSequenceInput};

pub mod config;
pub mod mutators;
pub mod sequence;

/// The result of a fuzzing campaign.
#[derive(Debug)]
pub struct FuzzResult {
    pub iterations: u64,
    pub crashes: Vec<Vec<u8>>,
}

/// A fuzzer configured to run against a deployed contract.
#[derive(Debug)]
pub struct Fuzzer {
    runner: EvmRunner,
    seeds: Vec<CallSequenceInput>,
    config: FuzzConfig,
    abi: alloy_json_abi::JsonAbi,
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
        let runner = EvmRunner::from_target(&artifact)?;
        let seeds = build_seeds(&artifact, config.sequence_length);
        let selectors: Vec<[u8; 4]> = artifact
            .abi
            .functions()
            .map(|f| f.selector().into())
            .collect();
        let abi = artifact.abi.clone();
        Ok(Self {
            runner,
            seeds,
            config,
            abi,
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

        let crashes = RefCell::new(Vec::new());
        let runner = &self.runner;

        let mut harness = |input: &CallSequenceInput| match runner.run_sequence(&input.calls) {
            Ok(res) if res.all_ok && res.property_triggered => {
                let bytes = input.target_bytes();
                crashes.borrow_mut().push(bytes.to_vec());
                libafl::executors::ExitKind::Crash
            }
            Ok(_) => libafl::executors::ExitKind::Ok,
            Err(_) => libafl::executors::ExitKind::Ok,
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
                SequenceInsertMutator::new(self.selectors.clone()),
                SequenceDeleteMutator,
                SequenceSpliceMutator,
                SequenceInterleaveMutator,
                SequenceHeadMutator,
                SequenceTailMutator,
                SequenceArgMutator::new(self.abi.clone()),
            )),
            std::num::NonZeroUsize::new(1).unwrap(),
        ));

        let mut manager = NopEventManager::new();
        let mut iterations = 0;
        for _ in 0..config.max_iters {
            if !crashes.borrow().is_empty() {
                break;
            }
            fuzzer.fuzz_one(&mut stages, &mut executor, &mut state, &mut manager)?;
            iterations += 1;
        }

        Ok(FuzzResult {
            iterations,
            crashes: crashes.into_inner(),
        })
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
            !result.crashes.is_empty(),
            "raptor should find at least one crash (dragon caught)"
        );
    }
}
