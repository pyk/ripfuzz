//! Campaign orchestration for invariant testing and maximization runs.

use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::Result;
use tracing::{error, info, instrument};

pub use crate::campaign::invariant::InvariantCampaign;
pub use crate::campaign::max::MaxCampaign;
pub use crate::campaign::session::CampaignSession;

use crate::commands::run::Args;
use crate::formatter;

mod invariant;
mod max;
mod session;

/// Campaign mode selected by the harness contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignKind {
    /// Validate `invariant_*` functions (the default when no `max_*` function
    /// is declared).
    Invariant,
    /// Maximize the single `max_*` function's return value.
    Max,
}

/// Run the campaign selected by the harness contract.
#[instrument(skip(args), fields(harness = ?args.harness, threads = args.threads, max_runs = args.max_runs))]
pub fn run(args: Args) -> Result<()> {
    let session = CampaignSession::new(args)?;
    match session.kind {
        CampaignKind::Invariant => InvariantCampaign::new(session)?.run(),
        CampaignKind::Max => MaxCampaign::new(session)?.run(),
    }
}

/// Log the start and finish of a campaign phase.
pub fn phase<T>(name: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    info!("[*] {name} ...");
    let started = Instant::now();
    let result = f();
    match &result {
        Ok(_) => info!(
            "[+] {name} in {}",
            formatter::duration(started.elapsed().as_secs_f64())
        ),
        Err(error) => error!("[!] {name} failed: {error:#}"),
    }
    result
}

/// Split `total` runs evenly across `workers`, one item per worker.
pub fn split_runs(total: u64, workers: usize) -> impl Iterator<Item = u64> {
    let base = total / workers as u64;
    let remainder = (total % workers as u64) as usize;
    (0..workers).map(move |i| if i < remainder { base + 1 } else { base })
}

/// Poll `handles` until every worker finishes, invoking `progress` each poll.
pub fn wait_for_workers<'a, T: 'a>(
    handles: impl IntoIterator<Item = &'a JoinHandle<T>>,
    mut progress: impl FnMut() -> Result<()>,
) -> Result<()> {
    let handles: Vec<&'a JoinHandle<T>> = handles.into_iter().collect();
    while handles.iter().any(|handle| !handle.is_finished()) {
        progress()?;
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(())
}
