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
    Fuzzer,
};
use libafl_bolts::{ownedref::OwnedMutSlice, rands::StdRand, tuples::tuple_list};

use crate::evm::EvmRunner;

pub fn run(runner: &EvmRunner, seeds: Vec<Vec<u8>>) -> anyhow::Result<()> {
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

    for seed in seeds {
        state
            .corpus_mut()
            .add(Testcase::new(BytesInput::from(seed)))?;
    }

    let scheduler = QueueScheduler::new();
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    let mut harness = |input: &BytesInput| {
        let bytes = input.target_bytes();
        match runner.run_sequence(&bytes) {
            Ok(true) => libafl::executors::ExitKind::Ok,
            Ok(false) => libafl::executors::ExitKind::Crash,
            Err(_) => libafl::executors::ExitKind::Crash,
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

    Ok(())
}
