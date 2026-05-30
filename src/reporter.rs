//! User-friendly console reporter for the Raptor CLI.
//!
//! Provides a [`Reporter`] that prints status messages to stderr with coloured
//! status prefixes. When running in a terminal, `begin` / `end` pairs replace
//! the previous line so the user only sees the final result.

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// ANSI sequence to return to the start of the current line and clear it.
const REPLACE_LINE: &str = "\r\x1b[K";

/// Console reporter for structured progress messages.
pub struct Reporter<W> {
    output: W,
    is_terminal: bool,
    start: Option<Instant>,
    message: Option<String>,
    last_lines: usize,
    /// Overrides the real elapsed time in tests.
    elapsed: Option<Duration>,
}

impl<W> std::fmt::Debug for Reporter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reporter")
            .field("is_terminal", &self.is_terminal)
            .field("start", &self.start)
            .field("message", &self.message)
            .field("last_lines", &self.last_lines)
            .field("elapsed", &self.elapsed)
            .finish_non_exhaustive()
    }
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
            last_lines: 0,
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
            last_lines: 0,
            elapsed: None,
        }
    }

    /// Print a one-off status line.
    ///
    /// The line is prefixed with a green `[*]` and always terminated with a
    /// newline.
    pub fn print(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let prefix = self.start_prefix();
        writeln!(
            self.output,
            "{prefix} {message}",
            message = message.as_ref()
        )
    }

    /// Print a one-off success line.
    ///
    /// The line is prefixed with a dim `[+]` and always terminated with a
    /// newline.
    pub fn print_success(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let prefix = self.success_prefix();
        writeln!(
            self.output,
            "{prefix} {message}",
            message = message.as_ref()
        )
    }

    /// Print a line without any status prefix.
    pub fn print_line(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        writeln!(self.output, "{message}", message = message.as_ref())
    }

    /// Replace the stored message without writing to the output.
    pub fn set_message(&mut self, message: impl AsRef<str>) {
        self.message = Some(message.as_ref().into());
    }

    /// Write a newline in terminal mode (no-op otherwise).
    pub fn new_line(&mut self) -> io::Result<()> {
        if self.is_terminal {
            writeln!(self.output)?;
        }
        Ok(())
    }

    /// Print a multi-line message, replacing the previous one in terminal mode.
    pub fn print_clearable(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let message = message.as_ref();
        let prev_lines = self.last_lines;
        if self.is_terminal && prev_lines > 0 {
            write!(self.output, "\r\x1b[K")?;
            for _ in 0..prev_lines - 1 {
                write!(self.output, "\x1b[A\r\x1b[K")?;
            }
            write!(self.output, "\x1b[A")?;
        }
        writeln!(self.output, "{message}")?;
        self.last_lines = message.matches('\n').count() + 1;
        Ok(())
    }

    /// End the current status, clearing any multi-line output and replacing the
    /// title line with the success message.
    pub fn clear_and_end(&mut self) -> io::Result<()> {
        let (Some(message), Some(start)) = (self.message.take(), self.start.take()) else {
            return Ok(());
        };
        let elapsed = self.elapsed.take().unwrap_or_else(|| start.elapsed());
        let seconds = elapsed.as_secs_f64();
        let clear_lines = self.last_lines;
        self.last_lines = 0;

        if self.is_terminal {
            if clear_lines > 0 {
                write!(self.output, "\r\x1b[K")?;
                for _ in 0..clear_lines + 1 {
                    write!(self.output, "\x1b[A\r\x1b[K")?;
                }
            } else {
                write!(self.output, "\x1b[A\r\x1b[K")?;
            }
        }

        let prefix = self.success_prefix();
        writeln!(self.output, "{prefix} {message} in {seconds:.2}s")
    }

    /// Print a progress line to stderr.
    ///
    /// The line is prefixed with a green `[*]`. In a terminal the
    /// line will be replaced by the next call to [`Self::end`].
    pub fn begin(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        if self.start.is_some() {
            return Ok(());
        }
        let message = message.as_ref();
        self.message = Some(message.into());
        self.start = Some(Instant::now());
        self.elapsed = None;
        let prefix = self.start_prefix();
        if self.is_terminal {
            write!(self.output, "{prefix} {message}")
        } else {
            writeln!(self.output, "{prefix} {message}")
        }
    }

    /// Replace the current line with a new progress message.
    ///
    /// Only has an effect when `self.is_terminal` is true and a matching
    /// [`Self::begin`] call was made. In non-terminal mode this is a no-op
    /// so logs do not get spammed.
    pub fn update(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let message = message.as_ref();
        if self.start.is_none() {
            return Ok(());
        }
        self.message = Some(message.into());
        if self.is_terminal {
            write!(self.output, "{REPLACE_LINE}")?;
            let prefix = self.start_prefix();
            write!(self.output, "{prefix} {message}")?;
        }
        Ok(())
    }

    /// Replace the current line with a progress message that includes a dimmed
    /// elapsed-time suffix.
    ///
    /// The elapsed time is rendered as `[{secs:.1}s]` in dim colour. The stored
    /// message is the base text (without the suffix) so that [`Self::end`] can
    /// append its own final timestamp without duplication.
    pub fn update_with_elapsed(
        &mut self,
        message: impl AsRef<str>,
        elapsed_secs: f64,
    ) -> io::Result<()> {
        let message = message.as_ref();
        if self.start.is_none() {
            return Ok(());
        }
        self.message = Some(message.into());
        if self.is_terminal {
            write!(self.output, "{REPLACE_LINE}")?;
            let prefix = self.start_prefix();
            write!(
                self.output,
                "{prefix} {message} {DIM}[{elapsed_secs:.1}s]{RESET}"
            )?;
        }
        Ok(())
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

        let prefix = self.success_prefix();
        writeln!(self.output, "{prefix} {message} in {seconds:.2}s")
    }

    /// Replace the line printed by the matching [`Self::begin`] call with a
    /// failure indicator.
    ///
    /// If no matching `begin` call was made, this is a no-op.
    pub fn fail(&mut self) -> io::Result<()> {
        let (Some(message), Some(start)) = (self.message.take(), self.start.take()) else {
            return Ok(());
        };
        let elapsed = self.elapsed.take().unwrap_or_else(|| start.elapsed());
        let seconds = elapsed.as_secs_f64();

        if self.is_terminal {
            write!(self.output, "{REPLACE_LINE}")?;
        }

        let prefix = self.fail_prefix();
        writeln!(self.output, "{prefix} {message} in {seconds:.2}s")
    }

    fn start_prefix(&self) -> String {
        if self.is_terminal {
            format!("{GREEN}[*]{RESET}")
        } else {
            "[*]".into()
        }
    }

    fn success_prefix(&self) -> String {
        if self.is_terminal {
            format!("{DIM}[+]{RESET}")
        } else {
            "[+]".into()
        }
    }

    fn fail_prefix(&self) -> String {
        if self.is_terminal {
            format!("{RED}[!]{RESET}")
        } else {
            "[!]".into()
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
            "[*] building project\n[+] building project in 2.00s\n"
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
        let prefix_start = format!("{GREEN}[*]{RESET}");
        let prefix_end = format!("{DIM}[+]{RESET}");
        assert_eq!(
            output,
            format!(
                "{prefix_start} building project{REPLACE_LINE}{prefix_end} building project in 2.00s\n"
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
    fn reporter_fail_without_begin_is_noop() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, false);
        reporter.fail().unwrap();
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
            "[*] first\n[+] first in 1.00s\n[*] second\n[+] second in 3.00s\n"
        );
    }

    #[test]
    fn reporter_update_terminal() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, true);
        reporter.begin("loading build artifacts").unwrap();
        reporter.update("loading build artifacts [1/12]").unwrap();
        reporter.elapsed = Some(Duration::from_secs_f64(2.0));
        reporter.end().unwrap();

        let output = String::from_utf8(buf).unwrap();
        let prefix_start = format!("{GREEN}[*]{RESET}");
        let prefix_end = format!("{DIM}[+]{RESET}");
        assert_eq!(
            output,
            format!(
                "{prefix_start} loading build artifacts\
                 {REPLACE_LINE}{prefix_start} loading build artifacts [1/12]\
                 {REPLACE_LINE}{prefix_end} loading build artifacts [1/12] in 2.00s\n"
            )
        );
    }

    #[test]
    fn reporter_update_non_terminal() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, false);
        reporter.begin("loading build artifacts").unwrap();
        reporter.update("loading build artifacts [1/12]").unwrap();
        reporter.elapsed = Some(Duration::from_secs_f64(2.0));
        reporter.end().unwrap();

        let output = String::from_utf8(buf).unwrap();
        // update does not write in non-terminal mode, but it still updates
        // the stored message so end() uses the latest text.
        assert_eq!(
            output,
            "[*] loading build artifacts\n[+] loading build artifacts [1/12] in 2.00s\n"
        );
    }

    #[test]
    fn reporter_multiple_updates_terminal() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, true);
        reporter.begin("loading build artifacts").unwrap();
        reporter.update("loading build artifacts [1/3]").unwrap();
        reporter.update("loading build artifacts [2/3]").unwrap();
        reporter.update("loading build artifacts [3/3]").unwrap();
        reporter.elapsed = Some(Duration::from_secs_f64(2.0));
        reporter.end().unwrap();

        let output = String::from_utf8(buf).unwrap();
        let prefix_start = format!("{GREEN}[*]{RESET}");
        let prefix_end = format!("{DIM}[+]{RESET}");
        assert_eq!(
            output,
            format!(
                "{prefix_start} loading build artifacts\
                 {REPLACE_LINE}{prefix_start} loading build artifacts [1/3]\
                 {REPLACE_LINE}{prefix_start} loading build artifacts [2/3]\
                 {REPLACE_LINE}{prefix_start} loading build artifacts [3/3]\
                 {REPLACE_LINE}{prefix_end} loading build artifacts [3/3] in 2.00s\n"
            )
        );
    }

    #[test]
    fn reporter_update_without_begin_is_noop() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, true);
        reporter.update("loading build artifacts [1/12]").unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn reporter_fail_non_terminal() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, false);
        reporter.begin("building project").unwrap();
        reporter.elapsed = Some(Duration::from_secs_f64(2.0));
        reporter.fail().unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            "[*] building project\n[!] building project in 2.00s\n"
        );
    }

    #[test]
    fn reporter_fail_terminal() {
        let mut buf = Vec::new();
        let mut reporter = Reporter::with_writer(&mut buf, true);
        reporter.begin("building project").unwrap();
        reporter.elapsed = Some(Duration::from_secs_f64(2.0));
        reporter.fail().unwrap();

        let output = String::from_utf8(buf).unwrap();
        let prefix_start = format!("{GREEN}[*]{RESET}");
        let prefix_fail = format!("{RED}[!]{RESET}");
        assert_eq!(
            output,
            format!(
                "{prefix_start} building project{REPLACE_LINE}{prefix_fail} building project in 2.00s\n"
            )
        );
    }
}
