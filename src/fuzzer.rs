use std::cell::RefCell;

use anyhow::Result;
use libafl::Fuzzer as _;
use libafl::{
    corpus::{Corpus, InMemoryCorpus, Testcase},
    events::SimpleEventManager,
    executors::InProcessExecutor,
    feedbacks::{CrashFeedback, MaxMapFeedback},
    fuzzer::StdFuzzer,
    inputs::{BytesInput, HasTargetBytes},
    monitors::SimplePrintingMonitor,
    mutators::{havoc_mutations, scheduled::HavocScheduledMutator},
    observers::StdMapObserver,
    schedulers::QueueScheduler,
    stages::StdMutationalStage,
    state::{HasCorpus, StdState},
};
use libafl_bolts::{ownedref::OwnedMutSlice, rands::StdRand, tuples::tuple_list};

use crate::contract::ContractArtifact;
use crate::evm::EvmRunner;

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
    seeds: Vec<Vec<u8>>,
}

impl Fuzzer {
    /// Deploy the artifact, generate seeds, and prepare the fuzzer.
    pub fn from_artifact(artifact: ContractArtifact) -> Result<Self> {
        let runner = EvmRunner::from_target(&artifact)?;
        let seeds = build_seeds(&artifact);
        Ok(Self { runner, seeds })
    }

    /// Run the fuzz loop and return a summary of the results.
    pub fn run(&self) -> Result<FuzzResult> {
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
            StdRand::with_seed(0),
            InMemoryCorpus::<BytesInput>::new(),
            InMemoryCorpus::new(),
            &mut feedback,
            &mut objective,
        )?;

        for seed in &self.seeds {
            state
                .corpus_mut()
                .add(Testcase::new(BytesInput::from(seed.clone())))?;
        }

        let scheduler = QueueScheduler::new();
        let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

        let crashes = RefCell::new(Vec::new());
        let runner = &self.runner;

        let mut harness = |input: &BytesInput| {
            let bytes = input.target_bytes();
            match runner.run_sequence(&bytes) {
                Ok(true) => libafl::executors::ExitKind::Ok,
                Ok(false) => {
                    crashes.borrow_mut().push(bytes.to_vec());
                    libafl::executors::ExitKind::Crash
                }
                Err(_) => {
                    crashes.borrow_mut().push(bytes.to_vec());
                    libafl::executors::ExitKind::Crash
                }
            }
        };

        let mut executor = InProcessExecutor::new(
            &mut harness,
            tuple_list!(observer),
            &mut fuzzer,
            &mut state,
            &mut SimpleEventManager::new(SimplePrintingMonitor::new()),
        )?;

        let mut stages = tuple_list!(StdMutationalStage::new(HavocScheduledMutator::new(
            havoc_mutations(),
        )));

        fuzzer.fuzz_loop_for(
            &mut stages,
            &mut executor,
            &mut state,
            &mut SimpleEventManager::new(SimplePrintingMonitor::new()),
            10_000,
        )?;

        Ok(FuzzResult {
            iterations: 10_000,
            crashes: crashes.into_inner(),
        })
    }
}

/// Build seed inputs from the contract ABI.
fn build_seeds(artifact: &ContractArtifact) -> Vec<Vec<u8>> {
    let mut seeds = Vec::new();

    for func in artifact.abi.functions() {
        let selector = func.selector();
        let mut seed = selector.to_vec();
        seed.resize(36, 0);
        seeds.push(seed);
    }

    // Add a combined seed with all functions in order
    let mut combined = Vec::new();
    for func in artifact.abi.functions() {
        let selector = func.selector();
        combined.extend_from_slice(selector.as_slice());
        combined.resize(combined.len() + 32, 0);
    }
    if !combined.is_empty() {
        seeds.push(combined);
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
        let expected = std::fs::read_to_string(
            "fixtures/basic-target/test/ConstructorRevertOutput.txt",
        )
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
}
