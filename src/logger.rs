//! Logging setup for the Raptor CLI.
//!
//! Provides a [`tracing`] subscriber that writes default formatted output
//! to a log file.

use std::fs;
use std::fs::File;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

/// Initialize the global tracing subscriber.
///
/// Writes formatted events to `log_file` at the given verbosity `level`.
pub fn init(log_file: &Path, level: tracing::Level) -> Result<()> {
    if let Some(parent) = log_file.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let file = File::create(log_file)?;

    let filter = match level {
        tracing::Level::ERROR => EnvFilter::new("raptor=error"),
        tracing::Level::WARN => EnvFilter::new("raptor=warn,revm=error"),
        tracing::Level::INFO => EnvFilter::new("raptor=info,revm=error"),
        tracing::Level::DEBUG => EnvFilter::new("raptor=debug,revm=warn"),
        tracing::Level::TRACE => EnvFilter::new("trace"),
    };

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .with_filter(filter),
        )
        .try_init()?;

    Ok(())
}
