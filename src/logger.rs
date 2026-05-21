//! Logging setup for the Raptor CLI.
//!
//! Provides a [`tracing`] subscriber that writes clean, structured output
//! to stdout with the log level as a prefix. Verbosity is controlled by
//! the CLI `-v` / `-vv` flags.

use std::io::IsTerminal;

use tracing::field::Visit;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::{self, FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

/// ANSI escape sequences - no external color crate.
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// Visitor that extracts the "message" field first, then buffers remaining
/// key-value pairs as `key=value` strings.
struct MessageFirstVisitor {
    message: String,
    data: Vec<(String, String)>,
}

impl MessageFirstVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            data: Vec::new(),
        }
    }
}

impl Visit for MessageFirstVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
            // Remove surrounding quotes that Debug adds to strings.
            if self.message.starts_with('"') && self.message.ends_with('"') {
                self.message = self.message[1..self.message.len() - 1].into();
            }
        } else {
            self.data
                .push((field.name().into(), format!("{:?}", value)));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.into();
        } else {
            self.data.push((field.name().into(), value.into()));
        }
    }
}

/// Custom event formatter that emits:
///
/// ```text
/// LEVEL Message text key=value key=value
/// ```
///
/// When the writer supports ANSI escapes, the prefix is bright green and
/// the data column is dimmed.
struct RaptorFormat;

impl<S, N> FormatEvent<S, N> for RaptorFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let has_ansi = writer.has_ansi_escapes();

        // 1. Prefix - red for errors, green for everything else.
        let level = event.metadata().level();
        let prefix_color = match *level {
            tracing::Level::ERROR => RED,
            _ => GREEN,
        };
        if has_ansi {
            write!(writer, "{}{level}{} ", prefix_color, RESET)?;
        } else {
            write!(writer, "{level} ")?;
        }

        // 2. Message + Data
        let mut visitor = MessageFirstVisitor::new();
        event.record(&mut visitor);

        write!(writer, "{}", visitor.message)?;

        if !visitor.data.is_empty() {
            if has_ansi {
                write!(writer, " {}", DIM)?;
            } else {
                write!(writer, " ")?;
            }
            for (i, (k, v)) in visitor.data.iter().enumerate() {
                if i > 0 {
                    write!(writer, " ")?;
                }
                write!(writer, "{k}={v}")?;
            }
            if has_ansi {
                write!(writer, "{}", RESET)?;
            }
        }

        writeln!(writer)
    }
}

/// Initialize the global tracing subscriber.
///
/// `level` is derived from the CLI verbosity flags (`-v`, `-vv`, etc.).
/// `None` means the user requested complete silence.
pub fn init(level: Option<tracing::Level>) {
    let filter = match level {
        None => EnvFilter::new("off"),
        Some(tracing::Level::ERROR) => EnvFilter::new("raptor=error"),
        Some(tracing::Level::WARN) => EnvFilter::new("raptor=warn,revm=error"),
        Some(tracing::Level::INFO) => EnvFilter::new("raptor=info,revm=error"),
        Some(tracing::Level::DEBUG) => EnvFilter::new("raptor=debug,revm=warn"),
        Some(tracing::Level::TRACE) => EnvFilter::new("trace"),
    };

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .event_format(RaptorFormat)
                .with_ansi(std::io::stdout().is_terminal())
                .with_filter(filter),
        )
        .init();
}
