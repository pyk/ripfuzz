//! Shared stop-on-revert event: the first reverted sequence that halts the
//! campaign.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::evm::Transaction;

/// The transaction sequence that triggered `--stop-on-revert`.
#[derive(Debug, Clone)]
pub struct StopEvent {
    pub transactions: Vec<Transaction>,
}

/// Thread-safe holder for the first stop event recorded by any fuzzer thread.
///
/// Cloning is cheap (shares the same inner state).
#[derive(Debug, Clone, Default)]
pub struct SharedStopEvent {
    inner: Arc<Mutex<Option<StopEvent>>>,
}

impl SharedStopEvent {
    /// Create an empty stop event holder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the first stop event; later events are ignored.
    pub fn set(&self, event: StopEvent) {
        let mut inner = self.inner.lock();
        if inner.is_none() {
            *inner = Some(event);
        }
    }

    /// The recorded stop event, if any.
    pub fn get(&self) -> Option<StopEvent> {
        self.inner.lock().clone()
    }
}
