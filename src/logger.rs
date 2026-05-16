//! Logging setup for the Raptor CLI.
//!
//! Provides a dual-layer [`tracing`] subscriber:
//! - A **user-facing** layer (stdout) for clean, unstructured output.
//! - A **diagnostic** layer (stderr) for internal debug / lifecycle logs
//!   gated by `-v` / `-vv`.

use tracing::Level;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::FormatFields;
use tracing_subscriber::prelude::*;

/// Custom event formatter for user-facing output.
///
/// Prints only the event fields (the message) with no timestamps, levels,
/// targets, or span context.
struct UserFormat;

impl<S, N> tracing_subscriber::fmt::FormatEvent<S, N> for UserFormat
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        ctx.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// Initialize the global tracing subscriber.
///
/// `level` is derived from the CLI verbosity flags (`-v`, `-vv`, etc.).
pub fn init(level: Option<Level>) {
    let user_layer = fmt::layer()
        .event_format(UserFormat)
        .with_filter(EnvFilter::new("raptor::user=info"));

    let diagnostic_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .event_format(fmt::format().compact().without_time().with_target(false))
        .with_filter(diagnostic_filter(level));

    tracing_subscriber::registry()
        .with(user_layer)
        .with(diagnostic_layer)
        .init();
}

/// Build the diagnostic [`EnvFilter`] from the CLI verbosity level.
fn diagnostic_filter(level: Option<Level>) -> EnvFilter {
    let directives = match level {
        None => "off",
        Some(Level::ERROR) => "error,raptor::user=off",
        Some(Level::WARN) => "warn,libafl=error,libafl_bolts=error,revm=error,raptor::user=off",
        // Default (no flags): keep diagnostics minimal; only raptor warnings and dependency errors.
        Some(Level::INFO) => {
            "raptor=warn,libafl=error,libafl_bolts=error,revm=error,raptor::user=off"
        }
        // -v: show raptor lifecycle info and dependency warnings.
        Some(Level::DEBUG) => {
            "raptor=info,libafl=warn,libafl_bolts=warn,revm=warn,raptor::user=off"
        }
        // -vv: full trace of everything.
        Some(Level::TRACE) => "trace,raptor::user=off",
    };
    EnvFilter::new(directives)
}
