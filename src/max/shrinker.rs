//! Shrinking of the best sequence to the shortest one preserving the value.
//!
//! [`Shrinker`] runs parallel shrinkers that delete random chunks of calls
//! from the best sequence. A candidate is valid when replaying it from a
//! clean chain keeps the final value at or above the target value, so every
//! accepted candidate is a full clean-state replay.
//!
//! Invariants (from the max ideas):
//!
//! - the sequence length never increases
//! - the final value never drops below the target value
//!
//! ```rust
//! use ripfuzz::max::Shrinker;
//!
//! // let shrinker = Shrinker::new().with_chain(chain);
//! // let shrunk = shrinker.shrink(&best_sequence)?;
//! ```

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy_primitives::Address;
use anyhow::{Context, Result};
use revm::primitives::Bytes;
use tracing::{error, info};

use crate::evm::{Chain, Transaction};
use crate::max::{Sequence, Value};

/// Interval between progress logs.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(3);

/// Interval between finished checks while waiting for the shrinkers.
const PROGRESS_TICK: Duration = Duration::from_millis(100);

/// Parallel shrinker for the best sequence.
///
/// The type carries its inputs as optional fields set via `with_*` builders;
/// `shrink` resolves them and errors on the missing ones.
#[derive(Clone, Debug, Default)]
pub struct Shrinker {
    chain: Option<Chain>,
    target: Option<Address>,
    deployer: Option<Address>,
    value_calldata: Option<Bytes>,
    target_value: Option<Value>,
    seed: Option<u64>,
    threads: Option<usize>,
    max_runs: Option<u64>,
    timeout: Option<Option<Duration>>,
}

impl Shrinker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the chain snapshot every shrinker clones from.
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

    /// Set the value the shrunk sequence must still reach.
    pub fn with_target_value(mut self, target_value: Value) -> Self {
        self.target_value = Some(target_value);
        self
    }

    /// Set the RNG seed.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set the number of shrinkers.
    pub fn with_threads(mut self, threads: usize) -> Self {
        self.threads = Some(threads);
        self
    }

    /// Set the maximum number of validation executions across all shrinkers.
    pub fn with_max_runs(mut self, max_runs: u64) -> Self {
        self.max_runs = Some(max_runs);
        self
    }

    /// Set the timeout after which shrinking stops.
    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Shrink the sequence while its final value stays at or above the
    /// target value, and return the shortest sequence found.
    pub fn shrink(self, sequence: &Sequence) -> Result<Sequence> {
        // 1. Short sequences are already minimal.
        if sequence.len() <= 1 {
            return Ok(sequence.clone());
        }

        // 2. Require the execution context.
        let execution = Execution {
            chain: self
                .chain
                .context("chain not set, call Shrinker::new().with_chain(..)")?,
            target: self
                .target
                .context("target not set, call Shrinker::new().with_target(..)")?,
            deployer: self
                .deployer
                .context("deployer not set, call Shrinker::new().with_deployer(..)")?,
            value_calldata: self
                .value_calldata
                .context("value calldata not set, call Shrinker::new().with_value_calldata(..)")?,
            target_value: self
                .target_value
                .context("target value not set, call Shrinker::new().with_target_value(..)")?,
            seed: self.seed.unwrap_or(0),
            threads: self.threads.unwrap_or(1),
            max_runs: self.max_runs.unwrap_or(0),
            timeout: self.timeout.unwrap_or_default(),
        };

        let start = Instant::now();
        let deadline = execution.timeout.map(|timeout| start + timeout);
        let shared = SharedShrink::new(sequence.clone(), deadline, execution.max_runs);
        let threads = match execution.threads {
            1 => "1 thread".to_string(),
            n => format!("{n} threads"),
        };
        let calls = match sequence.len() {
            1 => "1 call".to_string(),
            n => format!("{n} calls"),
        };
        info!(
            "shrinking started: {calls}, {threads}, {} runs, target {}",
            execution.max_runs, execution.target_value,
        );

        // 3. Spawn shrinkers over the shared current best.
        let mut handles = Vec::with_capacity(execution.threads);
        for thread_id in 0..execution.threads {
            // checkrs: allow(clone_in_loops)
            let execution = execution.clone();
            // checkrs: allow(clone_in_loops)
            let shared = shared.clone();
            handles.push((
                thread_id,
                std::thread::spawn(move || shrink_worker(&execution, &shared, thread_id)),
            ));
        }

        // 4. Log progress while the shrinkers run.
        let mut last_progress = Instant::now();
        while handles.iter().any(|(_, handle)| !handle.is_finished()) {
            std::thread::sleep(PROGRESS_TICK);
            if handles.iter().all(|(_, handle)| handle.is_finished()) {
                break;
            }
            if last_progress.elapsed() >= PROGRESS_INTERVAL {
                info!(
                    "shrinking progress: {} attempts, {} calls, {}s",
                    shared.attempts(),
                    shared.current_len(),
                    start.elapsed().as_secs(),
                );
                last_progress = Instant::now();
            }
        }

        // 5. Join the shrinkers and propagate failures.
        let mut failures: Vec<anyhow::Error> = Vec::new();
        for (thread_id, handle) in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    error!("shrinker {thread_id} failed: {err:#}");
                    failures.push(err);
                }
                Err(err) => {
                    error!("shrinker {thread_id} panicked: {err:?}");
                    failures.push(anyhow::anyhow!("shrinker {thread_id} panicked: {err:?}"));
                }
            }
        }
        if !failures.is_empty() {
            let count = failures.len();
            let first = failures.remove(0);
            return Err(first).with_context(|| format!("{count} shrinker(s) failed"));
        }

        // 6. Report the shortest sequence found.
        let shrunk = shared.current();
        info!(
            "shrinking finished: {} calls, {} attempts, {}s",
            shrunk.len(),
            shared.attempts(),
            start.elapsed().as_secs(),
        );
        Ok(shrunk)
    }
}

/// Resolved shrinker inputs for one run, an internal context that keeps the
/// worker signature compact.
#[derive(Clone, Debug)]
struct Execution {
    chain: Chain,
    target: Address,
    deployer: Address,
    value_calldata: Bytes,
    target_value: Value,
    seed: u64,
    threads: usize,
    max_runs: u64,
    timeout: Option<Duration>,
}

/// State shared across shrinkers.
#[derive(Debug, Clone)]
struct SharedShrink {
    current: Arc<Mutex<Sequence>>,
    attempts: Arc<AtomicU64>,
    deadline: Option<Instant>,
    max_runs: u64,
}

impl SharedShrink {
    /// Create shared state from the seed sequence and budget.
    fn new(current: Sequence, deadline: Option<Instant>, max_runs: u64) -> Self {
        Self {
            current: Arc::new(Mutex::new(current)),
            attempts: Arc::new(AtomicU64::new(0)),
            deadline,
            max_runs,
        }
    }

    /// Lock the current sequence, recovering from poisoning.
    fn lock_current(&self) -> std::sync::MutexGuard<'_, Sequence> {
        self.current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// A clone of the current shortest sequence.
    fn current(&self) -> Sequence {
        self.lock_current().clone()
    }

    /// The length of the current shortest sequence.
    fn current_len(&self) -> usize {
        self.lock_current().len()
    }

    /// Replace the current sequence when the candidate is shorter, returning
    /// whether it was accepted.
    fn update(&self, candidate: Sequence) -> bool {
        let mut current = self.lock_current();
        if candidate.len() < current.len() {
            *current = candidate;
            true
        } else {
            false
        }
    }

    /// Record one validation attempt, returning whether the budget remains.
    fn record_attempt(&self) -> bool {
        let attempts = self.attempts.fetch_add(1, Ordering::Relaxed) + 1;
        attempts <= self.max_runs
    }

    /// The number of validation attempts so far.
    fn attempts(&self) -> u64 {
        self.attempts.load(Ordering::Relaxed)
    }

    /// Whether the validation budget is exhausted.
    fn exhausted(&self) -> bool {
        self.attempts() >= self.max_runs
    }

    /// Whether the timeout has elapsed.
    fn timed_out(&self) -> bool {
        match self.deadline {
            Some(deadline) => Instant::now() >= deadline,
            None => false,
        }
    }
}

/// Delete random chunks from the current best until the budget is exhausted
/// or the sequence is fully shrunk.
fn shrink_worker(execution: &Execution, shared: &SharedShrink, thread_id: usize) -> Result<()> {
    let mut rng = fastrand::Rng::with_seed(execution.seed.wrapping_add(thread_id as u64));
    let value_tx = Transaction::new(execution.target).calldata(execution.value_calldata.clone());
    loop {
        if shared.exhausted() || shared.timed_out() {
            break;
        }
        let current = shared.current();
        if current.len() <= 1 {
            break;
        }

        // 1. Pick a random chunk of calls to delete.
        let len = current.len();
        let chunk_len = if rng.bool() { 1 } else { rng.usize(1..=len) };
        let start_pos = rng.usize(..len);
        let end_pos = (start_pos + chunk_len).min(len);
        let candidate = current.without(start_pos..end_pos);
        if candidate.is_empty() {
            continue;
        }
        if !shared.record_attempt() {
            break;
        }

        // 2. Validate the candidate on a clean chain clone.
        // checkrs: allow(clone_in_loops)
        let mut chain = execution.chain.clone();
        let transactions = candidate.transactions(execution.target, execution.deployer);
        chain.exec(&transactions)?;

        // 3. Measure the value after the candidate.
        let output = chain.exec(std::slice::from_ref(&value_tx))?;
        let result = output
            .results
            .first()
            .context("value call result missing")?;
        if !result.success {
            continue;
        }
        let value = Value::decode(result)?;

        // 4. Accept the candidate when the target value is preserved.
        if value >= execution.target_value {
            // checkrs: allow(clone_in_loops)
            if shared.update(candidate.clone()) {
                info!("shrunk sequence to {} calls", candidate.len());
            }
        }
    }
    Ok(())
}
