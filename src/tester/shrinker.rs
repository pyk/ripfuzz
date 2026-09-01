//! Shrinking of broken-invariant sequences to the shortest ones preserving
//! the broken invariant.
//!
//! [`Shrinker`] runs parallel shrinkers that delete random chunks of calls
//! from each broken invariant's sequence. A candidate is valid when replaying
//! it from a clean chain reproduces the same `rvm.bail` id, so every accepted
//! candidate is a full clean-state replay.
//!
//! Invariants:
//!
//! - the sequence length never increases
//! - the bail-emitting call still emits the exact same broken-invariant id
//!
//! ```rust,no_run
//! use alloy_primitives::Address;
//! use ripfuzz::tester::Shrinker;
//! use ripfuzz::{Chain, ChainConfig};
//!
//! # fn main() -> anyhow::Result<()> {
//! # let chain = Chain::empty(ChainConfig::default());
//! # let target = Address::ZERO;
//! # let deployer = Address::ZERO;
//! # let broken_invariants = Vec::new();
//! let shrunk = Shrinker::new()
//!     .with_chain(chain)
//!     .with_target(target)
//!     .with_deployer(deployer)
//!     .shrink(&broken_invariants)?;
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use alloy_primitives::Address;
use anyhow::{Context, Result};
use tracing::{error, info};

use crate::evm::Chain;
use crate::tester::{BrokenInvariant, Sequence};

/// Interval between progress logs.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(3);

/// Interval between finished checks while waiting for the shrinkers.
const PROGRESS_TICK: Duration = Duration::from_millis(100);

/// Parallel shrinker for broken-invariant sequences.
///
/// The type carries its inputs as optional fields set via `with_*` builders;
/// `shrink` resolves them and errors on the missing ones.
#[derive(Clone, Debug, Default)]
pub struct Shrinker {
    chain: Option<Chain>,
    target: Option<Address>,
    deployer: Option<Address>,
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

    /// Set the maximum number of validation executions per broken invariant
    /// across all shrinker threads.
    pub fn with_max_runs(mut self, max_runs: u64) -> Self {
        self.max_runs = Some(max_runs);
        self
    }

    /// Set the timeout after which shrinking stops.
    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Shrink every broken invariant's sequence while it still emits the
    /// exact same broken-invariant id, and return the shrunk broken
    /// invariants.
    pub fn shrink(self, broken_invariants: &[BrokenInvariant]) -> Result<Vec<BrokenInvariant>> {
        // 1. Require the execution context.
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
            seed: self.seed.unwrap_or(0),
            threads: self.threads.unwrap_or(1),
            max_runs: self.max_runs.unwrap_or(0),
            timeout: self.timeout.unwrap_or_default(),
        };

        // 2. Shrink each broken invariant independently with its own budget.
        let start = Instant::now();
        info!(
            broken_invariants = broken_invariants.len(),
            threads = execution.threads,
            runs = execution.max_runs,
            "shrinking started"
        );
        let mut shrunk = Vec::with_capacity(broken_invariants.len());
        for broken in broken_invariants.iter() {
            let shrunk_broken = shrink_one(&execution, broken)?;
            info!(
                id = %shrunk_broken.id(),
                initial_calls = broken.sequence().len(),
                final_calls = shrunk_broken.sequence().len(),
                "broken invariant minimized"
            );
            shrunk.push(shrunk_broken);
        }
        info!(
            broken_invariants = shrunk.len(),
            elapsed = start.elapsed().as_secs(),
            "shrinking finished"
        );
        Ok(shrunk)
    }
}

/// Shrink one broken invariant's sequence to the shortest one that still
/// reproduces the exact broken invariant.
fn shrink_one(execution: &Execution, broken: &BrokenInvariant) -> Result<BrokenInvariant> {
    // 1. Short sequences are already minimal.
    if broken.sequence().len() <= 1 {
        return Ok(broken.clone());
    }

    let start = Instant::now();
    let deadline = execution.timeout.map(|timeout| start + timeout);
    let shared = SharedShrink::new(broken.clone(), deadline, execution.max_runs);

    // 2. Spawn shrinkers over the shared current best.
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

    // 3. Log progress while the shrinkers run.
    let mut last_progress = Instant::now();
    while handles.iter().any(|(_, handle)| !handle.is_finished()) {
        std::thread::sleep(PROGRESS_TICK);
        if handles.iter().all(|(_, handle)| handle.is_finished()) {
            break;
        }
        if last_progress.elapsed() >= PROGRESS_INTERVAL {
            info!(
                id = %broken.id(),
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
    Ok(shrunk)
}

/// Resolved shrinker inputs for one run, an internal context that keeps the
/// worker signature compact.
#[derive(Clone, Debug)]
struct Execution {
    chain: Chain,
    target: Address,
    deployer: Address,
    seed: u64,
    threads: usize,
    max_runs: u64,
    timeout: Option<Duration>,
}

/// State shared across shrinkers for one broken invariant.
#[derive(Debug, Clone)]
struct SharedShrink {
    current: Arc<Mutex<BrokenInvariant>>,
    attempts: Arc<AtomicU64>,
    deadline: Option<Instant>,
    max_runs: u64,
}

impl SharedShrink {
    /// Create shared state from the seed broken invariant and budget.
    fn new(current: BrokenInvariant, deadline: Option<Instant>, max_runs: u64) -> Self {
        Self {
            current: Arc::new(Mutex::new(current)),
            attempts: Arc::new(AtomicU64::new(0)),
            deadline,
            max_runs,
        }
    }

    /// Lock the current broken invariant, recovering from poisoning.
    fn lock_current(&self) -> std::sync::MutexGuard<'_, BrokenInvariant> {
        self.current
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// A clone of the current shortest broken invariant.
    fn current(&self) -> BrokenInvariant {
        self.lock_current().clone()
    }

    /// The length of the current shortest sequence.
    fn current_len(&self) -> usize {
        self.lock_current().sequence().len()
    }

    /// Replace the current broken invariant when the candidate is shorter,
    /// returning whether it was accepted.
    fn update(&self, candidate: BrokenInvariant) -> bool {
        let mut current = self.lock_current();
        if candidate.sequence().len() < current.sequence().len() {
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

/// Whether replaying the candidate sequence on a clean chain still emits the
/// broken invariant's exact id. The sequence ends with the bail-emitting
/// call, so replaying it alone must reproduce the finding.
fn reproduces(
    execution: &Execution,
    sequence: &Sequence,
    broken: &BrokenInvariant,
) -> Result<bool> {
    // checkrs: allow(clone_in_loops) each candidate replays on a clean state
    let mut chain = execution.chain.clone();
    let transactions = sequence.transactions(execution.target, execution.deployer);
    let exec = chain.exec(&transactions)?;
    Ok(exec
        .broken_invariants
        .iter()
        .any(|per_tx| per_tx.iter().any(|report| report.id == broken.id())))
}

/// Delete random chunks from the current sequence until the budget is
/// exhausted or the sequence is fully shrunk.
fn shrink_worker(execution: &Execution, shared: &SharedShrink, thread_id: usize) -> Result<()> {
    let mut rng = fastrand::Rng::with_seed(execution.seed.wrapping_add(thread_id as u64));
    loop {
        if shared.exhausted() || shared.timed_out() {
            break;
        }
        let current = shared.current();
        if current.sequence().is_empty() {
            break;
        }

        // 1. Pick a random chunk of calls to delete.
        let len = current.sequence().len();
        let chunk_len = if rng.bool() { 1 } else { rng.usize(1..=len) };
        let start_pos = rng.usize(..len);
        let end_pos = (start_pos + chunk_len).min(len);
        let candidate = current.sequence().without(start_pos..end_pos);
        if !shared.record_attempt() {
            break;
        }

        // 2. Accept the candidate when the broken invariant still reproduces
        //    exactly on a clean replay.
        if reproduces(execution, &candidate, &current)? {
            // checkrs: allow(clone_in_loops) the accepted broken invariant must own its data
            let candidate_broken = current.clone().with_calls(candidate);
            let _ = shared.update(candidate_broken);
        }
    }
    Ok(())
}
