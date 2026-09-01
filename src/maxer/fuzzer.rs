//! Fuzzing of high-value harness sequences.
//!
//! [`Fuzzer`] spawns fuzzers (threads) that generate random handler-call
//! sequences, execute each on a clean chain clone, and track the best
//! sequence by final value.
//!
//! Stop conditions, checked between sequences:
//!
//! - the run budget is exhausted
//! - the timeout elapses
//! - the best value reaches the target value
//!
//! ```rust,no_run
//! use ripfuzz::maxer::{Corpus, Fuzzer, Sequence, Value};
//! use ripfuzz::{Chain, ChainConfig, SharedCoverage};
//! use alloy_primitives::U256;
//!
//! # let chain = Chain::empty(ChainConfig::default());
//! # let corpus = Corpus::new();
//! # let coverage = SharedCoverage::new();
//! # let value = Value::new(U256::from(1));
//! let best = Fuzzer::new()
//!     .with_chain(chain)
//!     .with_corpus(corpus)
//!     .with_coverage(coverage)
//!     .with_initial_value(value)
//!     .run()
//!     .unwrap();
//! ```

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};
use revm::primitives::Bytes;
use tracing::{error, info, warn};

use crate::evm::{Chain, SharedCoverage, Transaction};
use crate::maxer::{Best, Call, Corpus, Sequence, Value};

/// Interval between progress logs.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(3);

/// Interval between finished checks while waiting for the fuzzers.
const PROGRESS_TICK: Duration = Duration::from_millis(100);

/// Per-thread fuzzer that discovers high-value sequences.
/// Discovery-phase fuzzer that finds high-value sequences.
///
/// The type carries its inputs as optional fields set via `with_*` builders;
/// `run` resolves them and errors on the missing ones.
#[derive(Clone, Debug, Default)]
pub struct Fuzzer {
    chain: Option<Chain>,
    target: Option<Address>,
    deployer: Option<Address>,
    value_calldata: Option<Bytes>,
    handlers: Option<Vec<Function>>,
    corpus: Option<Corpus>,
    coverage: Option<SharedCoverage>,
    initial_value: Option<Value>,
    seed: Option<u64>,
    threads: Option<usize>,
    max_runs: Option<u64>,
    max_calls: Option<usize>,
    timeout: Option<Duration>,
    target_value: Option<Value>,
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

    /// Set the calldata that reads the harness value.
    pub fn with_value_calldata(mut self, calldata: Bytes) -> Self {
        self.value_calldata = Some(calldata);
        self
    }

    /// Set the fuzzable handler functions.
    pub fn with_handlers(mut self, handlers: Vec<Function>) -> Self {
        self.handlers = Some(handlers);
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

    /// Set the value measured right after setup.
    pub fn with_initial_value(mut self, initial_value: Value) -> Self {
        self.initial_value = Some(initial_value);
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
    pub fn with_max_calls(mut self, max_calls: usize) -> Self {
        self.max_calls = Some(max_calls);
        self
    }

    /// Set the timeout after which fuzzing stops.
    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the value at which fuzzing stops early.
    pub fn with_target_value(mut self, target_value: Option<Value>) -> Self {
        self.target_value = target_value;
        self
    }

    /// Run fuzzing and return the best sequence found.
    pub fn run(self) -> Result<Best> {
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
            value_calldata: self
                .value_calldata
                .context("value calldata not set, call Fuzzer::new().with_value_calldata(..)")?,
            handlers: self
                .handlers
                .context("handlers not set, call Fuzzer::new().with_handlers(..)")?,
            corpus: self
                .corpus
                .context("corpus not set, call Fuzzer::new().with_corpus(..)")?,
            coverage: self
                .coverage
                .context("coverage not set, call Fuzzer::new().with_coverage(..)")?,
            initial_value: self
                .initial_value
                .context("initial value not set, call Fuzzer::new().with_initial_value(..)")?,
            seed: self.seed.unwrap_or(0),
            threads: self.threads.unwrap_or(1),
            max_runs: self.max_runs.unwrap_or(0),
            max_calls: self.max_calls.unwrap_or(8),
            timeout: self.timeout,
            target_value: self.target_value,
        };

        // 2. Seed the shared state with the initial value and stop signals.
        //
        //    The replayed corpus may already hold a higher value than the
        //    initial one, e.g. when a previous campaign found a high value,
        //    seed the best from that entry so reruns reuse its snapshot
        //    instead of rediscovering it.
        let start = Instant::now();
        let deadline = execution.timeout.map(|timeout| start + timeout);
        let shared = Shared::new(
            execution
                .corpus
                .best_entry()
                .map(|(sequence, value, chain)| Best::with_chain(sequence, value, chain))
                .unwrap_or_else(|| Best::new(Sequence::empty(), execution.initial_value)),
            execution.target_value,
            deadline,
        );

        // 3. Skip fuzzing when the target value is already met.
        if let Some(target) = execution.target_value
            && shared.best_value() >= target
        {
            info!(
                "target value {target} already met with {}, skipping fuzzing",
                shared.best_value(),
            );
            return Ok(shared.into_best());
        }

        // 4. Skip fuzzing when the harness has no handler functions.
        if execution.handlers.is_empty() {
            warn!("no handler functions to fuzz, skipping fuzzing");
            return Ok(shared.into_best());
        }

        let threads = match execution.threads {
            1 => "1 thread".to_string(),
            n => format!("{n} threads"),
        };
        let timeout = match execution.timeout {
            Some(timeout) => format!("{}s timeout", timeout.as_secs()),
            None => "no timeout".to_string(),
        };
        info!(
            "fuzzing started: {threads}, {} runs, max {} calls, {timeout}",
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
                info!(
                    "fuzzing progress: {} runs, best {}, {} corpus, {} edges, {}s",
                    shared.runs(),
                    shared.best_value(),
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

        // 7. Report the best sequence found.
        let best = shared.into_best();
        if best.sequence().is_empty() {
            warn!("no sequence improved the initial value {}", best.value());
        } else {
            info!(
                "best sequence: value {}, {} calls, {}, {}s",
                best.value(),
                best.sequence().len(),
                best.sequence(),
                start.elapsed().as_secs(),
            );
        }
        Ok(best)
    }
}

/// Resolved fuzzer inputs for one run, an internal context that keeps the
/// worker signature from exploding into per-field parameters.
#[derive(Clone, Debug)]
struct Execution {
    chain: Chain,
    target: Address,
    deployer: Address,
    value_calldata: Bytes,
    handlers: Vec<Function>,
    corpus: Corpus,
    coverage: SharedCoverage,
    initial_value: Value,
    seed: u64,
    threads: usize,
    max_runs: u64,
    max_calls: usize,
    timeout: Option<Duration>,
    target_value: Option<Value>,
}

/// State shared across fuzzers.
#[derive(Debug, Clone)]
struct Shared {
    best: Arc<Mutex<Best>>,
    runs: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    deadline: Option<Instant>,
    target_value: Option<Value>,
}

impl Shared {
    /// Create shared state from the seed best and stop signals.
    fn new(best: Best, target_value: Option<Value>, deadline: Option<Instant>) -> Self {
        Self {
            best: Arc::new(Mutex::new(best)),
            runs: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            deadline,
            target_value,
        }
    }

    /// Lock the best tracker, recovering from poisoning.
    ///
    /// A panic in one fuzzer must not take the others down, and `Best` stays
    /// valid even when a fuzzer dies mid-update.
    fn lock_best(&self) -> std::sync::MutexGuard<'_, Best> {
        self.best.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Record the sequence when its value is strictly higher, returning
    /// whether it replaced the current best. Stops fuzzing when the target
    /// value is reached.
    fn consider(&self, sequence: Sequence, value: Value, chain: Chain) -> bool {
        let improved = self.lock_best().consider(sequence, value, chain);
        let target_reached = match self.target_value {
            Some(target) => value >= target,
            None => false,
        };
        if improved && target_reached {
            self.stop.store(true, Ordering::Relaxed);
        }
        improved
    }

    /// Record one executed sequence.
    fn record_run(&self) {
        self.runs.fetch_add(1, Ordering::Relaxed);
    }

    /// The number of sequences executed so far.
    fn runs(&self) -> u64 {
        self.runs.load(Ordering::Relaxed)
    }

    /// The value of the best sequence so far.
    fn best_value(&self) -> Value {
        self.lock_best().value()
    }

    /// Clone the current best sequence for reuse.
    ///
    /// Returns the sequence with the state after executing it, so a worker
    /// can extend the best state directly. Falls back to `None` while no
    /// sequence has improved the initial value or the snapshot is missing.
    fn best_base(&self) -> Option<(Sequence, Chain)> {
        let best = self.lock_best();
        let chain = best.chain()?;
        if best.sequence().is_empty() {
            None
        } else {
            Some((best.sequence().clone(), chain.clone()))
        }
    }

    /// Whether another fuzzer asked to stop.
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Whether the timeout has elapsed.
    fn timed_out(&self) -> bool {
        match self.deadline {
            Some(deadline) => Instant::now() >= deadline,
            None => false,
        }
    }

    /// Take the best sequence found.
    fn into_best(self) -> Best {
        self.lock_best().clone()
    }
}

/// Extend memoized snapshots with fresh calls until the thread budget is
/// exhausted or a stop condition fires.
///
/// The corpus is a prefix tree over states.
///
/// Each entry memoizes the chain state after its sequence, and a candidate
/// extends one snapshot with a single fresh call. Interesting results become
/// new snapshots.
///
/// This mirrors snapshot-based fuzzing with two benefits:
///
/// - intermediate states such as an approved token or a funded vault position
///   survive as corpus entries
/// - a dependent call only has to be sampled once from the right state
///   instead of the whole chain landing in one random sequence
fn worker(execution: &Execution, shared: &Shared, thread_id: usize, runs: u64) -> Result<()> {
    let mut rng = fastrand::Rng::with_seed(execution.seed.wrapping_add(thread_id as u64));
    let value_tx = Transaction::new(execution.target).calldata(execution.value_calldata.clone());
    let mut best_known = execution.initial_value;
    for _ in 0..runs {
        if shared.stopped() || shared.timed_out() {
            break;
        }
        shared.record_run();

        // 1. Pick the snapshot to extend.
        //
        //    The choice balances broad search and use of best findings
        //
        //    - an eighth of the runs execute a fresh random sequence from the
        //      initial state for broad exploration
        //    - the rest extend a memoized snapshot
        //    - among snapshot extensions, three quarters extend the current
        //      best and one quarter samples a corpus entry weighted by
        //      coverage gain and value
        //
        //    The best is the most promising prefix, so extending it keeps the
        //    climb alive even when the corpus churns under new coverage from
        //    decoy handlers.
        let fresh = rng.usize(..8) == 0;
        // checkrs: allow(clone_in_loops) the initial snapshot seeds every run
        let initial = execution.chain.clone();
        let (sequence, pending, base_chain) = if fresh {
            let sequence = Sequence::random(&mut rng, &execution.handlers, execution.max_calls)?;
            let pending = sequence.calls().to_vec();
            (sequence, pending, initial)
        } else {
            let (base_sequence, base_chain) = if rng.usize(..4) != 0 {
                shared.best_base().unwrap_or_else(|| {
                    (
                        Sequence::empty(),
                        // checkrs: allow(clone_in_loops)
                        initial.clone(),
                    )
                })
            } else {
                execution.corpus.random_base(&mut rng).unwrap_or_else(|| {
                    (
                        Sequence::empty(),
                        // checkrs: allow(clone_in_loops)
                        initial.clone(),
                    )
                })
            };
            let base_len = base_sequence.calls().len();
            if base_len >= execution.max_calls {
                // 2a. At the call limit the snapshot cannot be extended without
                //     exceeding it.
                //
                //     Mutate a random position and re-execute the whole
                //     sequence from the initial state, so internal noise can be
                //     repaired:
                //
                //     - a leading `noiseWrite` that consumes a slot is replaced
                //     - the value climb can still reach `max_calls` pure calls
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
        };

        // 2a. Execute the pending calls on a chain clone of the snapshot.
        // checkrs: allow(clone_in_loops)
        let mut chain = base_chain.clone();
        let transactions: Vec<Transaction> = pending
            .iter()
            .map(|call| call.transaction(execution.target, execution.deployer))
            .collect();
        let mut exec = chain.exec(&transactions)?;

        // 3. Merge execution coverage into the shared map.
        let coverage = exec
            .coverage
            .take()
            .context("execution coverage expected")?;
        let update = execution.coverage.merge(&coverage);

        // 4. Measure the value after the sequence.
        let output = chain.exec(std::slice::from_ref(&value_tx))?;
        let result = output
            .results
            .first()
            .context("value call result missing")?;
        if !result.success {
            continue;
        }
        let value = Value::decode(result)?;

        // 5. Record the sequence when it improves the best value.
        // checkrs: allow(clone_in_loops)
        let improved = shared.consider(sequence.clone(), value, chain.clone());
        if improved {
            info!(
                "new best sequence on thread {thread_id}: value {value}, {} calls, {sequence}",
                sequence.len(),
            );
        }

        // 6. Keep the snapshot when it is interesting.
        //
        //    A snapshot is kept when any of the following holds:
        //
        //    - it found new coverage
        //    - it beat the global best
        //    - it beat this worker's previous mark, even when still below the
        //      global best
        //
        //    The last case keeps value-only additions, so the mutator retains
        //    access to the argument combinations behind every value climb, not
        //    just coverage-new ones.
        let beats_local = value > best_known;
        if beats_local {
            best_known = value;
        }
        if update.is_interesting() || improved || beats_local {
            let new_edges =
                (update.new_edges + update.new_depths + update.new_reverts + update.new_jump_edges)
                    as u64;
            execution.corpus.add(sequence, value, new_edges, chain);
        }
    }
    Ok(())
}

/// Split the run budget evenly across fuzzers.
fn split_runs(runs: u64, threads: usize) -> Vec<u64> {
    let base = runs / threads as u64;
    let remainder = runs % threads as u64;
    (0..threads as u64)
        .map(|index| base + u64::from(index < remainder))
        .collect()
}
