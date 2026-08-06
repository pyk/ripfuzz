//! Logging setup for the Ripfuzz CLI.
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
use tracing_subscriber::fmt::format::FmtSpan;
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
        tracing::Level::ERROR => EnvFilter::new("ripfuzz=error"),
        tracing::Level::WARN => EnvFilter::new("ripfuzz=warn,revm=error"),
        tracing::Level::INFO => EnvFilter::new("ripfuzz=info,revm=error"),
        tracing::Level::DEBUG => EnvFilter::new("ripfuzz=debug,revm=warn"),
        tracing::Level::TRACE => EnvFilter::new("trace"),
    };

    let layer = fmt::layer()
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .with_span_events(FmtSpan::CLOSE);

    tracing_subscriber::registry()
        .with(layer.with_filter(filter))
        .try_init()?;

    Ok(())
}
