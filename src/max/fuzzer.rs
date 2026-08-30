//! Fuzzing of high-value harness sequences.
//!
//! [`Fuzzer`] spawns fuzzers (threads) that generate random handler-call
//! sequences, execute each on a clean chain clone, and track the best
//! sequence by final value. This is the discovery phase from the max ideas.
//!
//! Stop conditions, checked between sequences:
//!
//! - the run budget is exhausted
//! - the timeout elapses
//! - the best value reaches the target value
//!
//! ```rust
//! use ripfuzz::max::Fuzzer;
//!
//! // let fuzzer = Fuzzer::new(FuzzerConfig::new().chain(chain));
//! // let best = fuzzer.run()?;
//! ```

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use anyhow::{Context, Result};
use revm::primitives::Bytes;
use tracing::{error, info, warn};

use crate::evm::{Chain, SharedCoverage, Transaction};
use crate::max::{Best, Corpus, Sequence, Value};

/// Interval between progress logs.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(3);

/// Interval between finished checks while waiting for the fuzzers.
const PROGRESS_TICK: Duration = Duration::from_millis(100);

/// Fuzzer configuration, configured via a fluent builder API.
#[derive(Clone, Debug)]
pub struct FuzzerConfig {
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

impl FuzzerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            chain: Chain::default(),
            target: Address::ZERO,
            deployer: Address::ZERO,
            value_calldata: Bytes::new(),
            handlers: Vec::new(),
            corpus: Corpus::new(),
            coverage: SharedCoverage::new(),
            initial_value: Value::new(U256::ZERO),
            seed: 0,
            threads: 1,
            max_runs: 0,
            max_calls: 8,
            timeout: None,
            target_value: None,
        }
    }

    /// Set the chain snapshot every fuzzer clones from.
    pub fn chain(mut self, value: Chain) -> Self {
        self.chain = value;
        self
    }

    /// Set the deployed harness address.
    pub fn target(mut self, value: Address) -> Self {
        self.target = value;
        self
    }

    /// Set the account address used to send calls.
    pub fn deployer(mut self, value: Address) -> Self {
        self.deployer = value;
        self
    }

    /// Set the calldata that reads the harness value.
    pub fn value_calldata(mut self, value: Bytes) -> Self {
        self.value_calldata = value;
        self
    }

    /// Set the fuzzable handler functions.
    pub fn handlers(mut self, value: Vec<Function>) -> Self {
        self.handlers = value;
        self
    }

    /// Set the shared corpus of interesting sequences.
    pub fn corpus(mut self, value: Corpus) -> Self {
        self.corpus = value;
        self
    }

    /// Set the shared coverage map.
    pub fn coverage(mut self, value: SharedCoverage) -> Self {
        self.coverage = value;
        self
    }

    /// Set the value measured right after setup.
    pub fn initial_value(mut self, value: Value) -> Self {
        self.initial_value = value;
        self
    }

    /// Set the RNG seed.
    pub fn seed(mut self, value: u64) -> Self {
        self.seed = value;
        self
    }

    /// Set the number of fuzzers.
    pub fn threads(mut self, value: usize) -> Self {
        self.threads = value;
        self
    }

    /// Set the maximum number of sequences to run across all threads.
    pub fn max_runs(mut self, value: u64) -> Self {
        self.max_runs = value;
        self
    }

    /// Set the maximum number of handler calls per sequence.
    pub fn max_calls(mut self, value: usize) -> Self {
        self.max_calls = value;
        self
    }

    /// Set the timeout after which fuzzing stops.
    pub fn timeout(mut self, value: Option<Duration>) -> Self {
        self.timeout = value;
        self
    }

    /// Set the value at which fuzzing stops early.
    pub fn target_value(mut self, value: Option<Value>) -> Self {
        self.target_value = value;
        self
    }
}

impl Default for FuzzerConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-thread fuzzer that discovers high-value sequences.
///
/// Created via [`FuzzerConfig`] and run via [`Fuzzer::run`].
#[derive(Debug)]
pub struct Fuzzer {
    config: FuzzerConfig,
}

impl Fuzzer {
    /// Create a new fuzzer from the given config.
    pub fn new(config: FuzzerConfig) -> Self {
        Self { config }
    }

    /// Run fuzzing and return the best sequence found.
    pub fn run(self) -> Result<Best> {
        // 1. Seed the shared state with the initial value and stop signals.
        let start = Instant::now();
        let deadline = self.config.timeout.map(|timeout| start + timeout);
        let shared = Shared::new(
            Best::new(Sequence::empty(), self.config.initial_value),
            self.config.target_value,
            deadline,
        );

        // 2. Skip fuzzing when the target value is already met.
        if let Some(target) = self.config.target_value
            && shared.best_value() >= target
        {
            info!(
                value = %shared.best_value(),
                target = %target,
                "target value already met, skipping fuzzing"
            );
            return Ok(shared.into_best());
        }

        // 3. Skip fuzzing when the harness has no handler functions.
        if self.config.handlers.is_empty() {
            warn!("no handler functions to fuzz, skipping fuzzing");
            return Ok(shared.into_best());
        }

        info!(
            threads = self.config.threads,
            runs = self.config.max_runs,
            max_calls = self.config.max_calls,
            timeout = ?self.config.timeout,
            "fuzzing started"
        );

        // 4. Spawn fuzzers with split run budgets.
        // 4. Spawn fuzzers with split run budgets.
        let budgets = split_runs(self.config.max_runs, self.config.threads);
        let mut handles = Vec::with_capacity(budgets.len());
        for (thread_id, budget) in budgets.into_iter().enumerate() {
            // checkrs: allow(clone_in_loops)
            let config = self.config.clone();
            // checkrs: allow(clone_in_loops)
            let shared = shared.clone();
            handles.push((
                thread_id,
                std::thread::spawn(move || worker(&config, &shared, thread_id, budget)),
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
                    best = %shared.best_value(),
                    corpus = self.config.corpus.len(),
                    edges = self.config.coverage.edge_count(),
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

        // 7. Report the best sequence found.
        let best = shared.into_best();
        if best.sequence().is_empty() {
            warn!(value = %best.value(), "no sequence improved the initial value");
        } else {
            info!(
                value = %best.value(),
                calls = best.sequence().len(),
                sequence = %best.sequence(),
                elapsed = start.elapsed().as_secs(),
                "best sequence"
            );
        }
        Ok(best)
    }
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
    fn consider(&self, sequence: Sequence, value: Value) -> bool {
        let improved = self.lock_best().consider(sequence, value);
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

    /// Clone the current best sequence for exploitation.
    ///
    /// Returns `None` while no sequence has improved the initial value.
    fn best_sequence(&self) -> Option<Sequence> {
        let best = self.lock_best();
        if best.sequence().is_empty() {
            None
        } else {
            Some(best.sequence().clone())
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

/// Execute random sequences until the thread budget is exhausted or a stop
/// condition fires.
fn worker(config: &FuzzerConfig, shared: &Shared, thread_id: usize, runs: u64) -> Result<()> {
    let mut rng = fastrand::Rng::with_seed(config.seed.wrapping_add(thread_id as u64));
    let value_tx = Transaction::new(config.target).calldata(config.value_calldata.clone());
    for _ in 0..runs {
        if shared.stopped() || shared.timed_out() {
            break;
        }
        shared.record_run();

        // 1. Generate the next sequence. A quarter of the runs are fresh
        //    random sequences for broad exploration. The rest mutate a base
        //    sequence, split evenly between the current best and a corpus
        //    entry: the best is the most promising prefix, so extending it
        //    keeps the climb alive even when the corpus churns under new
        //    coverage from decoy handlers.
        let sequence = if rng.usize(..4) == 0 {
            Sequence::random(&mut rng, &config.handlers, config.max_calls)?
        } else {
            let base = if rng.usize(..2) == 0 {
                shared.best_sequence().unwrap_or_default()
            } else {
                config.corpus.random(&mut rng).unwrap_or_default()
            };
            let other = config.corpus.random(&mut rng).unwrap_or_default();
            base.mutate(&mut rng, &config.handlers, &other, config.max_calls)?
        };

        // 2. Execute the sequence on a clean chain clone.
        // checkrs: allow(clone_in_loops)
        let mut chain = config.chain.clone();
        let transactions = sequence.transactions(config.target, config.deployer);
        let mut exec = chain.exec(&transactions)?;

        // 3. Merge execution coverage into the shared map.
        let coverage = exec
            .coverage
            .take()
            .context("execution coverage expected")?;
        let update = config.coverage.merge(&coverage);

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
        let improved = shared.consider(sequence.clone(), value);
        if improved {
            info!(
                thread = thread_id,
                value = %value,
                calls = sequence.len(),
                sequence = %sequence,
                "new best sequence"
            );
        }

        // 6. Keep the sequence when it found new coverage or a new best value.
        if update.is_interesting() || improved {
            let new_edges =
                (update.new_edges + update.new_depths + update.new_reverts + update.new_jump_edges)
                    as u64;
            config.corpus.add(sequence, value, new_edges);
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
