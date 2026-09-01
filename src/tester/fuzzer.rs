//! Fuzzing of harness sequences to find failed assertions.
//!
//! [`Fuzzer`] spawns fuzzers (threads) that generate random handler-call
//! sequences, execute each call on a clean chain clone, and check assertions
//! after every call:
//!
//! - a handler call whose execution raises a Solidity `assert` panic
//!   (`Panic(0x01)`) is a failed assertion
//! - after each committed handler call, every `invariant_*` function runs on
//!   a throwaway clone, so invariant state is never committed, and a panic
//!   there is a failed assertion too
//!
//! Stop conditions, checked between sequences:
//!
//! - the run budget is exhausted
//! - the timeout elapses
//! - the requested number of distinct findings is collected
//!
//! ```rust,no_run
//! use ripfuzz::max::Sequence;
//! use ripfuzz::tester::{Corpus, Fuzzer, SharedFindings};
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
//!     .with_findings(SharedFindings::new(256))
//!     .run()
//!     .unwrap();
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};
use revm::primitives::Bytes;
use tracing::{error, info, warn};

use crate::evm::{Chain, CoverageUpdate, SharedCoverage, Transaction};
use crate::max::{Call, Sequence};
use crate::tester::{Corpus, Finding, SharedFindings};

/// Interval between progress logs.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(3);

/// Interval between finished checks while waiting for the fuzzers.
const PROGRESS_TICK: Duration = Duration::from_millis(100);

/// Per-thread fuzzer that discovers failed assertions.
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
    findings: Option<SharedFindings>,
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

    /// Set the shared findings collector.
    pub fn with_findings(mut self, findings: SharedFindings) -> Self {
        self.findings = Some(findings);
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

    /// Run fuzzing and return the findings collected.
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
            findings: self
                .findings
                .context("findings not set, call Fuzzer::new().with_findings(..)")?,
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

        info!(
            threads = execution.threads,
            runs = execution.max_runs,
            max_calls = execution.max_calls,
            invariants = execution.invariants.len(),
            timeout = ?execution.timeout,
            "fuzzing started"
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
                info!(
                    runs = shared.runs(),
                    findings = execution.findings.len(),
                    corpus = execution.corpus.len(),
                    edges = execution.coverage.edge_count(),
                    elapsed = start.elapsed().as_secs(),
                    "fuzzing progress"
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
                    error!(thread_id, "fuzzer failed: {err:#}");
                    failures.push(err);
                }
                Err(err) => {
                    error!(thread_id, ?err, "fuzzer panicked");
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
        let findings = execution.findings.findings();
        if findings.is_empty() {
            info!(
                runs = shared.runs(),
                elapsed = start.elapsed().as_secs(),
                "no failed assertions found"
            );
        } else {
            info!(
                findings = findings.len(),
                runs = shared.runs(),
                elapsed = start.elapsed().as_secs(),
                "fuzzing finished"
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
    findings: SharedFindings,
    seed: u64,
    threads: usize,
    max_runs: u64,
    max_calls: usize,
    timeout: Option<Duration>,
}

/// The fuzzer outcome: the number of executed sequences and the distinct
/// failed assertions found.
#[derive(Debug, Clone)]
pub struct Output {
    /// The number of sequences executed across all threads.
    pub runs: u64,
    /// The distinct failed assertions in discovery order.
    pub findings: Vec<Finding>,
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
            findings: execution.findings.findings(),
        }
    }
}

/// Extend memoized snapshots with fresh calls until the thread budget is
/// exhausted or a stop condition fires.
///
/// The corpus is a prefix tree over states, mirroring the max fuzzer. Each
/// entry memoizes the chain state after its sequence, and a candidate extends
/// one snapshot with a single fresh call. Assertions live in the states the
/// sequence passes through, so extending a state that already reached one
/// failure tends to reach related failures with fewer new calls.
fn worker(execution: &Execution, shared: &Shared, thread_id: usize, runs: u64) -> Result<()> {
    let mut rng = fastrand::Rng::with_seed(execution.seed.wrapping_add(thread_id as u64));
    for _ in 0..runs {
        if shared.stopped() || shared.timed_out() {
            break;
        }
        if execution.findings.is_full() {
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
            let sequence = Sequence::random(&mut rng, &execution.handlers, execution.max_calls)?;
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
                        let call = Call::random(&mut rng, function)?;
                        let mut calls = base_sequence.calls().to_vec();
                        let pos = rng.usize(..calls.len());
                        calls[pos] = call;
                        let sequence = Sequence::new(calls);
                        let pending = sequence.calls().to_vec();
                        (sequence, pending, initial)
                    } else {
                        let function = &execution.handlers[rng.usize(..execution.handlers.len())];
                        let call = Call::random(&mut rng, function)?;
                        let mut calls = base_sequence.calls().to_vec();
                        calls.push(call);
                        let sequence = Sequence::new(calls);
                        let pending = sequence.calls()[base_len..].to_vec();
                        (sequence, pending, base_chain)
                    }
                }
                None => {
                    let sequence =
                        Sequence::random(&mut rng, &execution.handlers, execution.max_calls)?;
                    let pending = sequence.calls().to_vec();
                    (sequence, pending, initial)
                }
            }
        };

        // 2. Execute the pending calls, checking assertions along the way.
        //
        //    The chain snapshot after the last committed handler call joins
        //    the corpus when it brought new coverage or reached a finding, so
        //    later runs can extend that state instead of rediscovering it.
        execute_sequence(execution, shared, thread_id, &sequence, pending, base_chain)?;
    }
    Ok(())
}

/// Execute the pending calls on a chain clone of the base snapshot and check
/// assertions after each one.
///
/// The assertion checks are:
///
/// - a handler call that raises a Solidity `assert` panic is recorded with
///   the calls before it, then the sequence continues on the pre-call state
/// - after every committed handler call, the `invariant_*` functions run on
///   a throwaway clone, so their state changes are never committed
fn execute_sequence(
    execution: &Execution,
    shared: &Shared,
    thread_id: usize,
    sequence: &Sequence,
    pending: Vec<Call>,
    base_chain: Chain,
) -> Result<()> {
    // checkrs: allow(clone_in_loops) every run needs its own snapshot
    let mut chain = base_chain;
    let prefix_len = sequence.calls().len() - pending.len();
    let mut new_edges = 0u64;
    let mut found = false;

    for (offset, call) in pending.iter().enumerate() {
        let index = prefix_len + offset;

        // 1. Commit the handler call on the working chain.
        let tx = call.transaction(execution.target, execution.deployer);
        let mut exec = chain.exec(std::slice::from_ref(&tx))?;
        let result = &exec.results[0];
        if let Some(coverage) = exec.coverage.take() {
            let update = execution.coverage.merge(&coverage);
            new_edges += score(&update);
        }

        // 2. Record a failed assertion raised by the handler.
        //
        //    Only Solidity `assert` panics count, other reverts are plain
        //    control flow, e.g. `require` guards.
        if result.is_assert_failure() {
            // checkrs: allow(clone_in_loops) the finding must own its data
            let trigger = call.function().clone();
            // checkrs: allow(clone_in_loops) the finding must own its data
            let output = result.output.clone().unwrap_or_default();
            let sequence_prefix = Sequence::new(sequence.calls()[..index].to_vec());
            let finding = Finding::new(sequence_prefix, trigger, output);
            if execution.findings.try_add(&finding) {
                info!(
                    thread = thread_id,
                    function = %finding.trigger().signature(),
                    reason = %finding.reason_display(),
                    calls = finding.sequence().len(),
                    sequence = %finding.sequence(),
                    "failed assertion"
                );
                found = true;
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
        for (function, result) in execution.invariants.iter().zip(exec.results.iter()) {
            if !result.is_assert_failure() {
                continue;
            }
            let sequence_prefix = Sequence::new(sequence.calls()[..=index].to_vec());
            // checkrs: allow(clone_in_loops) the finding must own its data
            let trigger = function.clone();
            // checkrs: allow(clone_in_loops) the finding must own its data
            let output = result.output.clone().unwrap_or_default();
            let finding = Finding::new(sequence_prefix, trigger, output);
            if execution.findings.try_add(&finding) {
                info!(
                    thread = thread_id,
                    function = %finding.trigger().signature(),
                    reason = %finding.reason_display(),
                    calls = finding.sequence().len(),
                    sequence = %finding.sequence(),
                    "failed assertion"
                );
                found = true;
            }
        }
    }

    // 4. Keep the sequence in the corpus when it is interesting.
    //
    //    New coverage and findings both make the final state a promising
    //    mutation base, and a finding stops the whole campaign when the
    //    findings collector is full.

    if new_edges > 0 || found {
        execution.corpus.add(sequence.clone(), new_edges, chain);
    }
    if execution.findings.is_full() {
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
