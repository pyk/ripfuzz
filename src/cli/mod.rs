//! CLI command definitions.

pub use crate::config::Config;

pub mod exec;
pub mod fetch;
pub mod init;
pub mod inspect;
pub mod max;
pub mod test;

/// Default thread count for commands that fuzz across threads.
///
/// Uses the available parallelism of the machine so campaigns scale across
/// all CPU cores by default, falling back to a single thread when the
/// runtime cannot report it.
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
