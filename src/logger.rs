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
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::prelude::*;

/// Format the current local time as `HH:MM:SS` for terminal log lines.
///
/// The full date stays available in the campaign log file, so the terminal
/// only needs the wall-clock time. `FormatTime` is implemented for function
/// pointers, so `simple_time` can be passed directly to `with_timer`.
fn simple_time(w: &mut Writer<'_>) -> std::fmt::Result {
    let now = jiff::Zoned::now();
    match jiff::fmt::strtime::format("%H:%M:%S", &now) {
        Ok(time) => w.write_str(&time),
        Err(_) => Err(std::fmt::Error),
    }
}

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

    // Terminal format: simple time, level, and message (module target hidden).
    // Cast to a fn pointer: `FormatTime` covers `fn(&mut Writer<'_>) -> fmt::Result`,
    // not the zero-sized fn item type.
    let stderr_layer = fmt::layer()
        .with_timer(simple_time as fn(&mut Writer<'_>) -> std::fmt::Result)
        .with_target(false)
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
