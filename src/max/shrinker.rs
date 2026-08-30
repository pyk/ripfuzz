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
//! // let shrinker = Shrinker::new(ShrinkerConfig::new().chain(chain));
//! // let shrunk = shrinker.shrink(&best_sequence)?;
//! ```

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy_primitives::{Address, U256};
use anyhow::{Context, Result};
use revm::primitives::Bytes;
use tracing::{error, info};

use crate::evm::{Chain, Transaction};
use crate::max::{Sequence, Value};

/// Interval between progress logs.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(3);

/// Interval between finished checks while waiting for the shrinkers.
const PROGRESS_TICK: Duration = Duration::from_millis(100);

/// Shrinker configuration, configured via a fluent builder API.
#[derive(Clone, Debug)]
pub struct ShrinkerConfig {
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

impl ShrinkerConfig {
    /// Create a new empty config.
    pub fn new() -> Self {
        Self {
            chain: Chain::default(),
            target: Address::ZERO,
            deployer: Address::ZERO,
            value_calldata: Bytes::new(),
            target_value: Value::new(U256::ZERO),
            seed: 0,
            threads: 1,
            max_runs: 0,
            timeout: None,
        }
    }

    /// Set the chain snapshot every shrinker clones from.
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

    /// Set the value the shrunk sequence must still reach.
    pub fn target_value(mut self, value: Value) -> Self {
        self.target_value = value;
        self
    }

    /// Set the RNG seed.
    pub fn seed(mut self, value: u64) -> Self {
        self.seed = value;
        self
    }

    /// Set the number of shrinkers.
    pub fn threads(mut self, value: usize) -> Self {
        self.threads = value;
        self
    }

    /// Set the maximum number of validation executions across all shrinkers.
    pub fn max_runs(mut self, value: u64) -> Self {
        self.max_runs = value;
        self
    }

    /// Set the timeout after which shrinking stops.
    pub fn timeout(mut self, value: Option<Duration>) -> Self {
        self.timeout = value;
        self
    }
}

impl Default for ShrinkerConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Parallel shrinker for the best sequence.
///
/// Created via [`ShrinkerConfig`] and run via [`Shrinker::shrink`].
#[derive(Debug)]
pub struct Shrinker {
    config: ShrinkerConfig,
}

impl Shrinker {
    /// Create a new shrinker from the given config.
    pub fn new(config: ShrinkerConfig) -> Self {
        Self { config }
    }

    /// Shrink the sequence while its final value stays at or above the
    /// target value, and return the shortest sequence found.
    pub fn shrink(self, sequence: &Sequence) -> Result<Sequence> {
        // 1. Short sequences are already minimal.
        if sequence.len() <= 1 {
            return Ok(sequence.clone());
        }
        let start = Instant::now();
        let deadline = self.config.timeout.map(|timeout| start + timeout);
        let shared = SharedShrink::new(sequence.clone(), deadline, self.config.max_runs);
        info!(
            calls = sequence.len(),
            threads = self.config.threads,
            runs = self.config.max_runs,
            target = %self.config.target_value,
            "shrinking started"
        );

        // 2. Spawn shrinkers over the shared current best.
        let mut handles = Vec::with_capacity(self.config.threads);
        for thread_id in 0..self.config.threads {
            // checkrs: allow(clone_in_loops)
            let config = self.config.clone();
            // checkrs: allow(clone_in_loops)
            let shared = shared.clone();
            handles.push((
                thread_id,
                std::thread::spawn(move || shrink_worker(&config, &shared, thread_id)),
            ));
        }

        // 3. Log progress while the shrinkers run.
        let mut last_progress = Instant::now();
        while handles.iter().any(|(_, handle)| !handle.is_finished()) {
            std::thread::sleep(PROGRESS_TICK);
            if handles.iter().all(|(_, handle)| handle.is_finished()) {
                break;
            }
            if last_progress.elapsed() >= PROGRESS_INTERVAL {
                info!(
                    attempts = shared.attempts(),
                    calls = shared.current_len(),
                    elapsed = start.elapsed().as_secs(),
                    "shrinking progress"
                );
                last_progress = Instant::now();
            }
        }

        // 4. Join the shrinkers and propagate failures.
        let mut failures: Vec<anyhow::Error> = Vec::new();
        for (thread_id, handle) in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    error!(thread_id, "shrinker failed: {err:#}");
                    failures.push(err);
                }
                Err(err) => {
                    error!(thread_id, ?err, "shrinker panicked");
                    failures.push(anyhow::anyhow!("shrinker {thread_id} panicked: {err:?}"));
                }
            }
        }
        if !failures.is_empty() {
            let count = failures.len();
            let first = failures.remove(0);
            return Err(first).with_context(|| format!("{count} shrinker(s) failed"));
        }

        // 5. Report the shortest sequence found.
        let shrunk = shared.current();
        info!(
            calls = shrunk.len(),
            attempts = shared.attempts(),
            elapsed = start.elapsed().as_secs(),
            "shrinking finished"
        );
        Ok(shrunk)
    }
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
fn shrink_worker(config: &ShrinkerConfig, shared: &SharedShrink, thread_id: usize) -> Result<()> {
    let mut rng = fastrand::Rng::with_seed(config.seed.wrapping_add(thread_id as u64));
    let value_tx = Transaction::new(config.target).calldata(config.value_calldata.clone());
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
        let mut chain = config.chain.clone();
        let transactions = candidate.transactions(config.target, config.deployer);
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
        if value >= config.target_value {
            // checkrs: allow(clone_in_loops)
            if shared.update(candidate.clone()) {
                info!(calls = candidate.len(), "shrunk sequence");
            }
        }
    }
    Ok(())
}
