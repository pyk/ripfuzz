//! Fuzzing of harness sequences to find broken invariants.
//!
//! [`Fuzzer`] spawns fuzzers (threads) that generate random handler-call
//! sequences, execute each call on a clean chain clone, and collect
//! `rvm.bail` reports after every call:
//!
//! - a handler call that emits `rvm.bail` is a broken invariant, and the call
//!   reverts so the sequence continues on the pre-call state
//! - after each committed handler call, every `invariant_*` function runs on
//!   a throwaway clone, so invariant state is never committed, and a report
//!   there is a broken invariant too
//!
//! Stop conditions, checked between sequences:
//!
//! - the run budget is exhausted
//! - the timeout elapses
//! - the requested number of distinct broken invariants is collected
//!
//! ```rust,no_run
//! use ripfuzz::tester::{Corpus, Fuzzer, SharedBrokenInvariants};
//! use ripfuzz::{Chain, ChainConfig, SharedCoverage};
//!
//! # let chain = Chain::empty(ChainConfig::default());
//! # let corpus = Corpus::new();
//! # let coverage = SharedCoverage::new();
//! # let handlers = Vec::new();
//! # let invariants = Vec::new();
//! let output = Fuzzer::new()
//!     .with_chain(chain)
//!     .with_corpus(corpus)
//!     .with_coverage(coverage)
//!     .with_handlers(handlers)
//!     .with_invariants(invariants)
//!     .with_broken_invariants(SharedBrokenInvariants::new(256))
//!     .run()
//!     .unwrap();
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};
use revm::primitives::Bytes;
use tracing::{error, info, warn};

use crate::evm::{Chain, CoverageUpdate, SharedCoverage, Transaction};
use crate::tester::{BrokenInvariant, Call, Corpus, Sequence, SharedBrokenInvariants};

/// Interval between progress logs.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(3);

/// Interval between finished checks while waiting for the fuzzers.
const PROGRESS_TICK: Duration = Duration::from_millis(100);

/// Per-thread fuzzer that discovers broken invariants.
///
/// The type carries its inputs as optional fields set via `with_*` builders;
/// `run` resolves them and errors on the missing ones.
#[derive(Clone, Debug, Default)]
pub struct Fuzzer {
    chain: Option<Chain>,
    target: Option<Address>,
    deployer: Option<Address>,
    handlers: Option<Vec<Function>>,
    invariants: Option<Vec<Function>>,
    corpus: Option<Corpus>,
    coverage: Option<SharedCoverage>,
    broken_invariants: Option<SharedBrokenInvariants>,
    seed: Option<u64>,
    threads: Option<usize>,
    max_runs: Option<u64>,
    max_calls: Option<usize>,
    timeout: Option<Duration>,
}

impl Fuzzer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the chain snapshot every fuzzer clones from.
    pub fn with_chain(mut self, chain: Chain) -> Self {
        self.chain = Some(chain);
        self
    }

    /// Set the deployed harness address.
    pub fn with_target(mut self, target: Address) -> Self {
        self.target = Some(target);
        self
    }

    /// Set the account address used to send calls.
    pub fn with_deployer(mut self, deployer: Address) -> Self {
        self.deployer = Some(deployer);
        self
    }

    /// Set the fuzzable handler functions.
    pub fn with_handlers(mut self, handlers: Vec<Function>) -> Self {
        self.handlers = Some(handlers);
        self
    }

    /// Set the `invariant_*` functions checked after each handler call.
    pub fn with_invariants(mut self, invariants: Vec<Function>) -> Self {
        self.invariants = Some(invariants);
        self
    }

    /// Set the shared corpus of interesting sequences.
    pub fn with_corpus(mut self, corpus: Corpus) -> Self {
        self.corpus = Some(corpus);
        self
    }

    /// Set the shared coverage map.
    pub fn with_coverage(mut self, coverage: SharedCoverage) -> Self {
        self.coverage = Some(coverage);
        self
    }

    /// Set the shared broken-invariant collector.
    pub fn with_broken_invariants(mut self, broken_invariants: SharedBrokenInvariants) -> Self {
        self.broken_invariants = Some(broken_invariants);
        self
    }

    /// Set the RNG seed.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set the number of fuzzers.
    pub fn with_threads(mut self, threads: usize) -> Self {
        self.threads = Some(threads);
        self
    }

    /// Set the maximum number of sequences to run across all threads.
    pub fn with_max_runs(mut self, max_runs: u64) -> Self {
        self.max_runs = Some(max_runs);
        self
    }

    /// Set the maximum number of handler calls per sequence.
    ///
    /// `invariant_*` checks are appended after each handler call and do not
    /// consume this budget.
    pub fn with_max_calls(mut self, max_calls: usize) -> Self {
        self.max_calls = Some(max_calls);
        self
    }

    /// Set the timeout after which fuzzing stops.
    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Run fuzzing and return the broken invariants collected.
    pub fn run(self) -> Result<Output> {
        // 1. Require the execution context.
        let execution = Execution {
            chain: self
                .chain
                .context("chain not set, call Fuzzer::new().with_chain(..)")?,
            target: self
                .target
                .context("target not set, call Fuzzer::new().with_target(..)")?,
            deployer: self
                .deployer
                .context("deployer not set, call Fuzzer::new().with_deployer(..)")?,
            handlers: self
                .handlers
                .context("handlers not set, call Fuzzer::new().with_handlers(..)")?,
            invariants: self
                .invariants
                .context("invariants not set, call Fuzzer::new().with_invariants(..)")?,
            corpus: self
                .corpus
                .context("corpus not set, call Fuzzer::new().with_corpus(..)")?,
            coverage: self
                .coverage
                .context("coverage not set, call Fuzzer::new().with_coverage(..)")?,
            broken_invariants: self.broken_invariants.context(
                "broken invariants not set, call Fuzzer::new().with_broken_invariants(..)",
            )?,
            seed: self.seed.unwrap_or(0),
            threads: self.threads.unwrap_or(1),
            max_runs: self.max_runs.unwrap_or(0),
            max_calls: self.max_calls.unwrap_or(8),
            timeout: self.timeout,
        };

        // 2. Seed the shared stop signals.
        let start = Instant::now();
        let deadline = execution.timeout.map(|timeout| start + timeout);
        let shared = Shared::new(deadline);

        // 3. Skip fuzzing when the harness has no handler functions.
        if execution.handlers.is_empty() {
            warn!("no handler functions to fuzz, skipping fuzzing");
            return Ok(shared.finish(&execution));
        }

        let threads = match execution.threads {
            1 => "1 thread".to_string(),
            n => format!("{n} threads"),
        };
        let invariants = match execution.invariants.len() {
            1 => "1 invariant".to_string(),
            n => format!("{n} invariants"),
        };
        let timeout = match execution.timeout {
            Some(timeout) => format!("{}s timeout", timeout.as_secs()),
            None => "no timeout".to_string(),
        };
        info!(
            "fuzzing started: {threads}, {} runs, max {} calls, {invariants}, {timeout}",
            execution.max_runs, execution.max_calls,
        );

        // 4. Spawn fuzzers with split run budgets.
        let budgets = split_runs(execution.max_runs, execution.threads);
        let mut handles = Vec::with_capacity(budgets.len());
        for (thread_id, budget) in budgets.into_iter().enumerate() {
            // checkrs: allow(clone_in_loops)
            let execution = execution.clone();
            // checkrs: allow(clone_in_loops)
            let shared = shared.clone();
            handles.push((
                thread_id,
                std::thread::spawn(move || worker(&execution, &shared, thread_id, budget)),
            ));
        }

        // 5. Log progress while the fuzzers run.
        let mut last_progress = Instant::now();
        while handles.iter().any(|(_, handle)| !handle.is_finished()) {
            std::thread::sleep(PROGRESS_TICK);
            if handles.iter().all(|(_, handle)| handle.is_finished()) {
                break;
            }
            if last_progress.elapsed() >= PROGRESS_INTERVAL {
                let findings = match execution.broken_invariants.len() {
                    1 => "1 broken invariant".to_string(),
                    n => format!("{n} broken invariants"),
                };
                info!(
                    "fuzzing progress: {} runs, {findings}, {} corpus, {} edges, {}s",
                    shared.runs(),
                    execution.corpus.len(),
                    execution.coverage.edge_count(),
                    start.elapsed().as_secs(),
                );
                last_progress = Instant::now();
            }
        }

        // 6. Join the fuzzers and propagate failures.
        let mut failures: Vec<anyhow::Error> = Vec::new();
        for (thread_id, handle) in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    error!("fuzzer {thread_id} failed: {err:#}");
                    failures.push(err);
                }
                Err(err) => {
                    error!("fuzzer {thread_id} panicked: {err:?}");
                    failures.push(anyhow::anyhow!("fuzzer {thread_id} panicked: {err:?}"));
                }
            }
        }
        if !failures.is_empty() {
            let count = failures.len();
            let first = failures.remove(0);
            return Err(first).with_context(|| format!("{count} fuzzer(s) failed"));
        }

        // 7. Report the campaign outcome.
        let broken_invariants = execution.broken_invariants.all();
        if broken_invariants.is_empty() {
            info!(
                "no broken invariants found after {} runs in {}s",
                shared.runs(),
                start.elapsed().as_secs(),
            );
        } else {
            let findings = match broken_invariants.len() {
                1 => "1 broken invariant".to_string(),
                n => format!("{n} broken invariants"),
            };
            info!(
                "fuzzing finished: {findings}, {} runs, {}s",
                shared.runs(),
                start.elapsed().as_secs(),
            );
        }
        Ok(shared.finish(&execution))
    }
}

/// Resolved fuzzer inputs for one run, an internal context that keeps the
/// worker signature from exploding into per-field parameters.
#[derive(Clone, Debug)]
struct Execution {
    chain: Chain,
    target: Address,
    deployer: Address,
    handlers: Vec<Function>,
    invariants: Vec<Function>,
    corpus: Corpus,
    coverage: SharedCoverage,
    broken_invariants: SharedBrokenInvariants,
    seed: u64,
    threads: usize,
    max_runs: u64,
    max_calls: usize,
    timeout: Option<Duration>,
}

/// The fuzzer outcome: the number of executed sequences and the distinct
/// broken invariants found.
#[derive(Debug, Clone)]
pub struct Output {
    /// The number of sequences executed across all threads.
    pub runs: u64,
    /// The distinct broken invariants in discovery order.
    pub broken_invariants: Vec<BrokenInvariant>,
}

/// State shared across fuzzers.
#[derive(Debug, Clone)]
struct Shared {
    runs: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl Shared {
    /// Create shared state from the deadline.
    fn new(deadline: Option<Instant>) -> Self {
        Self {
            runs: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            deadline,
        }
    }

    /// Record one executed sequence.
    fn record_run(&self) {
        self.runs.fetch_add(1, Ordering::Relaxed);
    }

    /// The number of sequences executed so far.
    fn runs(&self) -> u64 {
        self.runs.load(Ordering::Relaxed)
    }

    /// Whether another fuzzer asked to stop.
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Ask every fuzzer to stop.
    fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Whether the timeout has elapsed.
    fn timed_out(&self) -> bool {
        match self.deadline {
            Some(deadline) => Instant::now() >= deadline,
            None => false,
        }
    }

    /// Take the fuzzer outcome.
    fn finish(&self, execution: &Execution) -> Output {
        Output {
            runs: self.runs(),
            broken_invariants: execution.broken_invariants.all(),
        }
    }
}

/// Extend memoized snapshots with fresh calls until the thread budget is
/// exhausted or a stop condition fires.
///
/// The corpus is a prefix tree over states, mirroring the max fuzzer. Each
/// entry memoizes the chain state after its sequence, and a candidate extends
/// one snapshot with a single fresh call. Broken invariants live in the
/// states the sequence passes through, so extending a state that already
/// broke one tends to break related invariants with fewer new calls.
fn worker(execution: &Execution, shared: &Shared, thread_id: usize, runs: u64) -> Result<()> {
    let mut rng = fastrand::Rng::with_seed(execution.seed.wrapping_add(thread_id as u64));
    for _ in 0..runs {
        if shared.stopped() || shared.timed_out() {
            break;
        }
        if execution.broken_invariants.is_full() {
            shared.request_stop();
            break;
        }
        shared.record_run();

        // 1. Pick the snapshot to extend.
        //
        //    An eighth of the runs execute a fresh random sequence from the
        //    initial state for broad exploration, the rest extend a memoized
        //    snapshot sampled by coverage gain.
        let fresh = rng.usize(..8) == 0;
        // checkrs: allow(clone_in_loops) the initial snapshot seeds every run
        let initial = execution.chain.clone();
        let (sequence, pending, base_chain) = if fresh {
            let sequence = Sequence::random(
                &mut rng,
                &execution.handlers,
                execution.max_calls,
                execution.corpus.literals(),
            )?;
            let pending = sequence.calls().to_vec();
            (sequence, pending, initial)
        } else {
            match execution.corpus.random_base(&mut rng) {
                Some((base_sequence, base_chain)) => {
                    let base_len = base_sequence.calls().len();
                    if base_len >= execution.max_calls {
                        // 1a. At the call limit the snapshot cannot be extended
                        //     without exceeding it, so mutate a random position
                        //     and re-execute the whole sequence from the initial
                        //     state.
                        let function = &execution.handlers[rng.usize(..execution.handlers.len())];
                        let call = Call::random(&mut rng, function, execution.corpus.literals())?;
                        let mut calls = base_sequence.calls().to_vec();
                        let pos = rng.usize(..calls.len());
                        calls[pos] = call;
                        let sequence = Sequence::new(calls);
                        let pending = sequence.calls().to_vec();
                        (sequence, pending, initial)
                    } else {
                        let function = &execution.handlers[rng.usize(..execution.handlers.len())];
                        let call = Call::random(&mut rng, function, execution.corpus.literals())?;
                        let mut calls = base_sequence.calls().to_vec();
                        calls.push(call);
                        let sequence = Sequence::new(calls);
                        let pending = sequence.calls()[base_len..].to_vec();
                        (sequence, pending, base_chain)
                    }
                }
                None => {
                    let sequence = Sequence::random(
                        &mut rng,
                        &execution.handlers,
                        execution.max_calls,
                        execution.corpus.literals(),
                    )?;
                    let pending = sequence.calls().to_vec();
                    (sequence, pending, initial)
                }
            }
        };

        // 2. Execute the pending calls, checking for broken invariants along
        //    the way.
        //
        //    The chain snapshot after the last committed handler call joins
        //    the corpus when it brought new coverage, so later runs can extend
        //    that state instead of rediscovering it.
        execute_sequence(execution, shared, thread_id, &sequence, pending, base_chain)?;
    }
    Ok(())
}

/// Execute the pending calls on a chain clone of the base snapshot and check
/// for broken invariants after each one.
///
/// The checks are:
///
/// - a handler call that emits `rvm.bail` is recorded with the calls before
///   it, and the call reverts so the sequence continues on the pre-call state
/// - after every committed handler call, the `invariant_*` functions run on
///   a throwaway clone, so their state changes are never committed
fn execute_sequence(
    execution: &Execution,
    shared: &Shared,
    _thread_id: usize,
    sequence: &Sequence,
    pending: Vec<Call>,
    base_chain: Chain,
) -> Result<()> {
    // checkrs: allow(clone_in_loops) every run needs its own snapshot
    let mut chain = base_chain;
    let prefix_len = sequence.calls().len() - pending.len();
    let mut new_edges = 0u64;

    for (offset, call) in pending.iter().enumerate() {
        let index = prefix_len + offset;

        // 1. Commit the handler call on the working chain.
        let tx = call.transaction(execution.target, execution.deployer);
        let mut exec = chain.exec(std::slice::from_ref(&tx))?;
        if let Some(coverage) = exec.coverage.take() {
            let update = execution.coverage.merge(&coverage);
            new_edges += score(&update);
        }

        // 2. Record reports emitted via `rvm.bail` in the handler. The
        //    sequence ends with the bail-emitting handler call.
        if !exec.broken_invariants.is_empty() {
            for report in &exec.broken_invariants[0] {
                let sequence_prefix = Sequence::new(sequence.calls()[..=index].to_vec());
                let broken = BrokenInvariant::new()
                    .with_calls(sequence_prefix)
                    .with_id(&report.id)
                    .with_description(&report.description);
                if execution.broken_invariants.try_add(&broken) {
                    info!("new broken invariant {}", broken.id());
                }
            }
        }

        // 3. Check the invariants on a throwaway clone of the post-call
        //    state, skipping the clone when there is nothing to check.
        if execution.invariants.is_empty() {
            continue;
        }
        // checkrs: allow(clone_in_loops) the invariant state must not commit
        let mut invariant_chain = chain.clone();
        let transactions: Vec<Transaction> = execution
            .invariants
            .iter()
            .map(|function| invariant_transaction(function, execution.target, execution.deployer))
            .collect();
        let mut exec = invariant_chain.exec(&transactions)?;
        if let Some(coverage) = exec.coverage.take() {
            let update = execution.coverage.merge(&coverage);
            new_edges += score(&update);
        }
        for (idx, function) in execution.invariants.iter().enumerate() {
            // 3a. Record reports emitted during invariants. The sequence ends
            //     with the invariant check call that bailed.
            if idx < exec.broken_invariants.len() {
                for report in &exec.broken_invariants[idx] {
                    let mut calls = sequence.calls()[..=index].to_vec();
                    // checkrs: allow(clone_in_loops) the broken invariant must own its data
                    calls.push(Call::new(function.clone(), DynSolValue::Tuple(vec![])));
                    let sequence_prefix = Sequence::new(calls);
                    let broken = BrokenInvariant::new()
                        .with_calls(sequence_prefix)
                        .with_id(&report.id)
                        .with_description(&report.description);
                    if execution.broken_invariants.try_add(&broken) {
                        info!("new broken invariant {}", broken.id());
                    }
                }
            }
        }
    }

    // 4. Keep the sequence in the corpus when it brought new coverage.
    //
    //    Broken invariants are collected separately. Re-adding a sequence
    //    only because it rediscovered a known invariant would grow the
    //    corpus on every campaign of a simple harness.

    if new_edges > 0 {
        execution.corpus.add(sequence.clone(), new_edges, chain);
    }
    if execution.broken_invariants.is_full() {
        shared.request_stop();
    }
    Ok(())
}

/// Sum the coverage counters of one merge update, the same score the max
/// corpus uses for eviction.
fn score(update: &CoverageUpdate) -> u64 {
    (update.new_edges + update.new_depths + update.new_reverts + update.new_jump_edges) as u64
}

/// Build the transaction that calls an `invariant_*` function.
///
/// Invariants take no arguments, so the calldata is just the selector.
fn invariant_transaction(function: &Function, target: Address, caller: Address) -> Transaction {
    Transaction::new(target)
        .caller(caller)
        .calldata(Bytes::from(function.selector().as_slice().to_vec()))
}

/// Split the run budget evenly across fuzzers.
fn split_runs(runs: u64, threads: usize) -> Vec<u64> {
    let base = runs / threads as u64;
    let remainder = runs % threads as u64;
    (0..threads as u64)
        .map(|index| base + u64::from(index < remainder))
        .collect()
}
