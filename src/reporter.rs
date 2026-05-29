//! User-friendly console reporter for the Raptor CLI.
//!
//! Provides a [`Reporter`] that prints status messages to stderr with a green
//! `raptor` prefix. When running in a terminal, `begin` / `end` pairs replace
//! the previous line so the user only sees the final result.

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

const GREEN: &str = "\x1b[32m";
const RESET: &str = "\x1b[0m";

/// ANSI sequence to move up one line, return to the start, and clear it.
const REPLACE_LINE: &str = "\x1b[1A\r\x1b[2K";

/// Console reporter for structured progress messages.
pub struct Reporter<W> {
    output: W,
    is_terminal: bool,
    start: Option<Instant>,
    message: Option<String>,
    /// Overrides the real elapsed time in tests.
    elapsed: Option<Duration>,
}

impl Default for Reporter<io::Stderr> {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter<io::Stderr> {
    /// Create a reporter that writes to the standard error stream.
    ///
    /// ANSI colours and line replacement are enabled only when stderr is a
    /// terminal.
    pub fn new() -> Self {
        let stderr = io::stderr();
        let is_terminal = stderr.is_terminal();
        Self {
            output: stderr,
            is_terminal,
            start: None,
            message: None,
            elapsed: None,
        }
    }
}

impl<W: Write> Reporter<W> {
    /// Create a reporter with an arbitrary writer.
    ///
    /// `is_terminal` controls whether ANSI escape sequences are emitted.
    /// This is useful for testing or for redirecting output to a file.
    pub fn with_writer(output: W, is_terminal: bool) -> Self {
        Self {
            output,
            is_terminal,
            start: None,
            message: None,
            elapsed: None,
        }
    }

    /// Print a progress line to stderr.
    ///
    /// The line is prefixed with a green `raptor` label. In a terminal the
    /// line will be replaced by the next call to [`Self::end`].
    pub fn begin(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let message = message.as_ref();
        self.message = Some(message.into());
        self.start = Some(Instant::now());
        self.elapsed = None;
        let prefix = self.prefix();
        writeln!(self.output, "{prefix} {message}")
    }

    /// Replace the line printed by the matching [`Self::begin`] call.
    ///
    /// If no matching `begin` call was made, this is a no-op.
    pub fn end(&mut self) -> io::Result<()> {
        let (Some(message), Some(start)) = (self.message.take(), self.start.take()) else {
            return Ok(());
        };
        let elapsed = self.elapsed.take().unwrap_or_else(|| start.elapsed());
        let seconds = elapsed.as_secs_f64();

        if self.is_terminal {
            write!(self.output, "{REPLACE_LINE}")?;
        }

        let prefix = self.prefix();
        writeln!(self.output, "{prefix} {message} ... done in {seconds:.2}s")
    }

    fn prefix(&self) -> String {
        if self.is_terminal {
            format!("{GREEN}raptor{RESET}")
        } else {
            "raptor".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reporter_non_terminal() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, false);
        reporter.begin("building project").unwrap();
        reporter.elapsed = Some(Duration::from_secs_f64(2.0));
        reporter.end().unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            "raptor building project\nraptor building project ... done in 2.00s\n"
        );
    }

    #[test]
    fn reporter_terminal() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, true);
        reporter.begin("building project").unwrap();
        reporter.elapsed = Some(Duration::from_secs_f64(2.0));
        reporter.end().unwrap();

        let output = String::from_utf8(buf).unwrap();
        let prefix = format!("{GREEN}raptor{RESET}");
        assert_eq!(
            output,
            format!(
                "{prefix} building project\n{REPLACE_LINE}{prefix} building project ... done in 2.00s\n"
            )
        );
    }

    #[test]
    fn reporter_end_without_begin_is_noop() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, false);
        reporter.end().unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn reporter_begin_can_be_reused() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, false);
        reporter.begin("first").unwrap();
        reporter.elapsed = Some(Duration::from_secs_f64(1.0));
        reporter.end().unwrap();
        reporter.begin("second").unwrap();
        reporter.elapsed = Some(Duration::from_secs_f64(3.0));
        reporter.end().unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            "raptor first\nraptor first ... done in 1.00s\nraptor second\nraptor second ... done in 3.00s\n"
        );
    }
}
