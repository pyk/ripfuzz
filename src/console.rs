//! User-friendly console output for the Ripfuzz CLI.
//!
//! Provides a [`Console`] that prints status messages to stderr with coloured
//! status prefixes. When running in a terminal, `begin` / `end` pairs replace
//! the previous line so the user only sees the final result.

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use tracing::{error, info};

use crate::formatter;

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// ANSI sequence to return to the start of the current line and clear it.
const REPLACE_LINE: &str = "\r\x1b[K";

/// Console for structured progress messages.
pub struct Console<W> {
    output: W,
    is_terminal: bool,
    start: Option<Instant>,
    message: Option<String>,
    /// Overrides the real elapsed time in tests.
    elapsed: Option<Duration>,
    /// When true, all output methods are no-ops.
    disabled: bool,
}

impl<W> std::fmt::Debug for Console<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Console")
            .field("is_terminal", &self.is_terminal)
            .field("start", &self.start)
            .field("message", &self.message)
            .field("elapsed", &self.elapsed)
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl Default for Console<io::Stderr> {
    fn default() -> Self {
        Self::new()
    }
}

impl Console<io::Stderr> {
    /// Create a console that writes to the standard error stream.
    ///
    /// ANSI colours and line replacement are enabled only when stderr is a
    /// terminal.
    pub fn new() -> Self {
        let stderr = io::stderr();
        let is_terminal = stderr.is_terminal();
        Self::with_writer(stderr, is_terminal)
    }
}

impl<W: Write> Console<W> {
    /// Create a console with an arbitrary writer.
    ///
    /// `is_terminal` controls whether ANSI escape sequences are emitted.
    /// This is useful for testing or for redirecting output to a file.
    fn with_writer(output: W, is_terminal: bool) -> Self {
        Self {
            output,
            is_terminal,
            start: None,
            message: None,
            elapsed: None,
            disabled: false,
        }
    }

    /// Enable or disable all console output.
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    /// Print a one-off status line.
    ///
    /// The line is prefixed with a green `[*]` and always terminated with a
    /// newline.
    pub fn print(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let message = message.as_ref();
        info!("{message}");
        if self.disabled {
            return Ok(());
        }
        let prefix = self.start_prefix();
        writeln!(self.output, "{prefix} {message}")
    }

    /// Print a one-off failure line.
    ///
    /// The line is prefixed with a red `[!]` and always terminated with a
    /// newline.
    pub fn print_fail(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let message = message.as_ref();
        error!("{message}");
        if self.disabled {
            return Ok(());
        }
        let prefix = self.fail_prefix();
        writeln!(self.output, "{prefix} {message}")
    }

    /// Print a one-off success line.
    ///
    /// The line is prefixed with a dim `[+]` and always terminated with a
    /// newline.
    pub fn print_success(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let message = message.as_ref();
        info!("{message}");
        if self.disabled {
            return Ok(());
        }
        let prefix = self.success_prefix();
        writeln!(self.output, "{prefix} {message}")
    }

    /// Print a line without any status prefix.
    pub fn print_line(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        writeln!(self.output, "{message}", message = message.as_ref())
    }

    /// Write a blank line.
    pub fn new_line(&mut self) -> io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        writeln!(self.output)
    }

    /// Print a periodic progress line.
    ///
    /// The line is prefixed with a dim `[~]` and always terminated with a
    /// newline. Unlike [`Self::update`], this never replaces a previous line.
    pub fn print_progress(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let prefix = self.progress_prefix();
        writeln!(
            self.output,
            "{prefix} {message}",
            message = message.as_ref()
        )
    }

    /// Print a progress line to stderr.
    ///
    /// The line is prefixed with a green `[*]`. In a terminal the
    /// line will be replaced by the next call to [`Self::end`].
    pub fn begin(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let message = message.as_ref();
        info!("{message}");
        if self.disabled {
            return Ok(());
        }
        if self.start.is_some() {
            return Ok(());
        }
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
        if self.disabled {
            return Ok(());
        }
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

    /// Replace the line printed by the matching [`Self::begin`] call.
    ///
    /// If no matching `begin` call was made, this is a no-op.
    pub fn end(&mut self) -> io::Result<()> {
        if self.disabled {
            self.message = None;
            self.start = None;
            self.elapsed = None;
            return Ok(());
        }
        let (Some(message), Some(start)) = (self.message.take(), self.start.take()) else {
            return Ok(());
        };
        let elapsed = self.elapsed.take().unwrap_or_else(|| start.elapsed());
        let seconds = elapsed.as_secs_f64();
        let duration = formatter::duration(seconds);
        info!("{message} in {duration}");

        if self.is_terminal {
            write!(self.output, "{REPLACE_LINE}")?;
        }

        let prefix = self.success_prefix();
        writeln!(self.output, "{prefix} {message} in {duration}")
    }

    /// Replace the line printed by the matching [`Self::begin`] call with a
    /// failure message.
    ///
    /// If no matching `begin` call was made, this is a no-op.
    pub fn end_fail(&mut self, message: impl AsRef<str>) -> io::Result<()> {
        let message = message.as_ref();
        if self.disabled {
            self.message = None;
            self.start = None;
            self.elapsed = None;
            error!("{message}");
            return Ok(());
        }
        let (Some(_), Some(_)) = (self.message.take(), self.start.take()) else {
            error!("{message}");
            return Ok(());
        };
        self.elapsed = None;
        error!("{message}");

        if self.is_terminal {
            write!(self.output, "{REPLACE_LINE}")?;
        }

        let prefix = self.fail_prefix();
        writeln!(self.output, "{prefix} {message}")
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

    fn progress_prefix(&self) -> String {
        if self.is_terminal {
            format!("{DIM}[~]{RESET}")
        } else {
            "[~]".into()
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
    fn console_non_terminal() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, false);
        console.begin("building project").unwrap();
        console.elapsed = Some(Duration::from_secs_f64(2.0));
        console.end().unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            "[*] building project\n[+] building project in 2.00s\n"
        );
    }

    #[test]
    fn console_terminal() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, true);
        console.begin("building project").unwrap();
        console.elapsed = Some(Duration::from_secs_f64(2.0));
        console.end().unwrap();

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
    fn console_end_without_begin_is_noop() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, false);
        console.end().unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn console_begin_can_be_reused() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, false);
        console.begin("first").unwrap();
        console.elapsed = Some(Duration::from_secs_f64(1.0));
        console.end().unwrap();
        console.begin("second").unwrap();
        console.elapsed = Some(Duration::from_secs_f64(3.0));
        console.end().unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            "[*] first\n[+] first in 1.00s\n[*] second\n[+] second in 3.00s\n"
        );
    }

    #[test]
    fn console_update_terminal() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, true);
        console.begin("loading build artifacts").unwrap();
        console.update("loading build artifacts [1/12]").unwrap();
        console.elapsed = Some(Duration::from_secs_f64(2.0));
        console.end().unwrap();

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
    fn console_update_non_terminal() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, false);
        console.begin("loading build artifacts").unwrap();
        console.update("loading build artifacts [1/12]").unwrap();
        console.elapsed = Some(Duration::from_secs_f64(2.0));
        console.end().unwrap();

        let output = String::from_utf8(buf).unwrap();
        // update does not write in non-terminal mode, but it still updates
        // the stored message so end() uses the latest text.
        assert_eq!(
            output,
            "[*] loading build artifacts\n[+] loading build artifacts [1/12] in 2.00s\n"
        );
    }

    #[test]
    fn console_multiple_updates_terminal() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, true);
        console.begin("loading build artifacts").unwrap();
        console.update("loading build artifacts [1/3]").unwrap();
        console.update("loading build artifacts [2/3]").unwrap();
        console.update("loading build artifacts [3/3]").unwrap();
        console.elapsed = Some(Duration::from_secs_f64(2.0));
        console.end().unwrap();

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
    fn console_update_without_begin_is_noop() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, true);
        console.update("loading build artifacts [1/12]").unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn console_end_fail_non_terminal() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, false);
        console.begin("deploying contract").unwrap();
        console.end_fail("failed to deploy contract").unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert_eq!(
            output,
            "[*] deploying contract\n[!] failed to deploy contract\n"
        );
    }

    #[test]
    fn console_end_fail_terminal() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, true);
        console.begin("deploying contract").unwrap();
        console.end_fail("failed to deploy contract").unwrap();

        let output = String::from_utf8(buf).unwrap();
        let prefix_start = format!("{GREEN}[*]{RESET}");
        let prefix_fail = format!("{RED}[!]{RESET}");
        assert_eq!(
            output,
            format!(
                "{prefix_start} deploying contract\
                {REPLACE_LINE}{prefix_fail} failed to deploy contract\n"
            )
        );
    }

    #[test]
    fn console_end_fail_without_begin_is_noop() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, false);
        console.end_fail("failed to deploy contract").unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn console_print_success() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, true);
        console.print_success("done").unwrap();

        let output = String::from_utf8(buf).unwrap();
        let prefix = format!("{DIM}[+]{RESET}");
        assert_eq!(output, format!("{prefix} done\n"));
    }

    #[test]
    fn console_print_progress_terminal() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, true);
        console.print_progress("fuzzing | 1,234 runs").unwrap();

        let output = String::from_utf8(buf).unwrap();
        let prefix = format!("{DIM}[~]{RESET}");
        assert_eq!(output, format!("{prefix} fuzzing | 1,234 runs\n"));
    }

    #[test]
    fn console_print_progress_non_terminal() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, false);
        console.print_progress("fuzzing | 1,234 runs").unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output, "[~] fuzzing | 1,234 runs\n");
    }

    #[test]
    fn console_new_line_writes_blank_line() {
        let mut buf = Vec::new();
        let mut console = Console::with_writer(&mut buf, false);
        console.print_line("stats").unwrap();
        console.new_line().unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output, "stats\n\n");
    }
}
