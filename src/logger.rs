//! Logging setup for the Ripfuzz CLI.
//!
//! Installs a default-format stderr layer and, unless disabled, a file layer.

use std::fs;
use std::fs::File;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::prelude::*;

/// Initialize the global tracing subscriber.
///
/// Terminal output is written to stderr in the default fmt format. When
/// `disable_log` is false, a formatted log file is also written at `log_file`.
pub fn init(disable_log: bool, log_file: &Path, level: tracing::Level) -> Result<()> {
    if disable_log {
        return Ok(());
    }

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

    // Default fmt format: timestamp, level, target, and message.
    let stderr_layer = fmt::layer()
        .with_ansi(std::io::stderr().is_terminal())
        .with_writer(std::io::stderr);

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .with_span_events(FmtSpan::CLOSE);

    tracing_subscriber::registry()
        .with(stderr_layer.with_filter(filter.clone()))
        .with(file_layer.with_filter(filter))
        .try_init()?;

    Ok(())
}
