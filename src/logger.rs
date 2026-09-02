//! Logging setup for the Ripfuzz CLI.
//!
//! [`Logger`] installs a compact stderr layer and a log file. Terminal
//! timestamps use local `HH:MM:SS.mmm`. Terminal lines omit `request`, print
//! `url` as the origin only, and shorten long `error` values. The log file
//! keeps every field.
//!
//! By default the log file is `{root}/.ripfuzz/logs/{unix-timestamp}-{id}.log`,
//! matching execution-trace naming.
//!
//! ```rust,no_run
//! use ripfuzz::logger::Logger;
//!
//! Logger::new()
//!     .with_root(std::path::Path::new("."))
//!     .with_level(tracing::Level::INFO)
//!     .init()
//!     .unwrap();
//! ```

use std::fmt;
use std::fs;
use std::fs::File;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use tracing::field::Field;
use tracing::field::Visit;
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::field::VisitOutput;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt as tracing_fmt;
use tracing_subscriber::fmt::format::DefaultVisitor;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::format::FormatFields;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::prelude::*;

/// Field formatter for stderr: drop bulky payloads and secret-bearing URL paths.
struct ConsoleFields;

impl ConsoleFields {
    /// Keep `scheme://host[:port]`; strip path, query, fragment, and userinfo.
    fn origin(url: &str) -> String {
        let Some(scheme_end) = url.find("://") else {
            return url.to_string();
        };
        let after_scheme = scheme_end + 3;
        let rest = &url[after_scheme..];
        let hostport = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        let hostport = match hostport.rsplit_once('@') {
            Some((_, host)) => host,
            None => hostport,
        };
        format!("{}{hostport}", &url[..after_scheme])
    }

    /// Keep the short prefix of a long RPC error; full text stays in the log file.
    fn compact_error(value: &str) -> &str {
        let Some((head, tail)) = value.split_once(": ") else {
            return value;
        };
        if tail.starts_with("http://")
            || tail.starts_with("https://")
            || tail.contains('{')
            || value.len() > 80
        {
            head
        } else {
            value
        }
    }
}

impl<'writer> FormatFields<'writer> for ConsoleFields {
    fn format_fields<R: RecordFields>(&self, writer: Writer<'writer>, fields: R) -> fmt::Result {
        let mut visitor = ConsoleVisitor {
            inner: DefaultVisitor::new(writer, true),
        };
        fields.record(&mut visitor);
        visitor.inner.finish()
    }
}

struct ConsoleVisitor<'a> {
    inner: DefaultVisitor<'a>,
}

impl ConsoleVisitor<'_> {
    fn record_text(&mut self, field: &Field, value: &str) {
        match field.name() {
            "request" => {}
            "url" => self
                .inner
                .record_debug(field, &format_args!("{}", ConsoleFields::origin(value))),
            "error" => self.inner.record_debug(
                field,
                &format_args!("{}", ConsoleFields::compact_error(value)),
            ),
            _ => self.inner.record_str(field, value),
        }
    }
}

impl Visit for ConsoleVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_text(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "request" => {}
            "url" | "error" => {
                let rendered = format!("{value:?}");
                let value = rendered
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(&rendered);
                self.record_text(field, value);
            }
            _ => self.inner.record_debug(field, value),
        }
    }
}

/// Format the current local time as `HH:MM:SS.mmm` for terminal log lines.
///
/// The full date stays available in the log file, so the terminal only needs
/// the wall-clock time. `FormatTime` is implemented for function pointers, so
/// `simple_time` can be passed directly to `with_timer`.
fn simple_time(w: &mut Writer<'_>) -> fmt::Result {
    let now = jiff::Zoned::now();
    write!(
        w,
        "{:02}:{:02}:{:02}.{:03}",
        now.hour(),
        now.minute(),
        now.second(),
        now.millisecond()
    )
}

/// Short unique id for a log file name, matching execution-trace ids.
fn log_id() -> String {
    let uuid: String = uuid::Uuid::new_v4().into();
    uuid.split('-').next().unwrap_or_default().to_owned()
}

/// Destination of the log-file layer.
#[derive(Debug)]
enum LogFile {
    /// Default `{root}/.ripfuzz/logs` path.
    Default,
    /// Explicit path.
    Path(PathBuf),
    /// No log file at all.
    Off,
}

/// Installs compact stderr logging and a timestamped log file under the
/// project root.
#[derive(Debug)]
pub struct Logger {
    root: PathBuf,
    quiet: bool,
    level: tracing::Level,
    log_file: LogFile,
    disabled: bool,
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}

impl Logger {
    /// Create a logger that writes under the working directory until
    /// `with_root` selects another project root.
    pub fn new() -> Self {
        Self {
            root: PathBuf::from("."),
            quiet: false,
            level: tracing::Level::INFO,
            log_file: LogFile::Default,
            disabled: false,
        }
    }

    /// Set the project root the log file lives under.
    pub fn with_root(mut self, root: impl AsRef<Path>) -> Self {
        self.root = root.as_ref().to_path_buf();
        self
    }

    /// Suppress terminal log output.
    pub fn with_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// Set the log verbosity level.
    pub fn with_level(mut self, level: tracing::Level) -> Self {
        self.level = level;
        self
    }

    /// Write the log file to an explicit path instead of `.ripfuzz/logs`.
    pub fn with_log_file(mut self, path: impl AsRef<Path>) -> Self {
        self.log_file = LogFile::Path(path.as_ref().to_path_buf());
        self
    }

    /// Skip log-file creation and write terminal output only.
    pub fn disable_log_file(mut self) -> Self {
        self.log_file = LogFile::Off;
        self
    }

    /// Skip subscriber setup and log-file creation.
    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Install the global tracing subscriber.
    ///
    /// Terminal output is written to stderr unless `quiet` is set. A formatted
    /// log file is also written unless logging is disabled or the file layer
    /// is off.
    pub fn init(self) -> Result<()> {
        // 1. Skip subscriber setup when logging is disabled.
        if self.disabled {
            return Ok(());
        }

        // 2. Create the log file unless the file layer is off.
        let file = match &self.log_file {
            LogFile::Off => None,
            LogFile::Path(path) => Some(open_log_file(path)?),
            LogFile::Default => {
                let timestamp = jiff::Timestamp::now().as_second();
                let path = self
                    .root
                    .join(".ripfuzz")
                    .join("logs")
                    .join(format!("{timestamp}-{}.log", log_id()));
                Some(open_log_file(&path)?)
            }
        };

        // 3. Build the log-level filter.
        let filter = match self.level {
            tracing::Level::ERROR => EnvFilter::new("ripfuzz=error"),
            tracing::Level::WARN => EnvFilter::new("ripfuzz=warn,revm=error"),
            tracing::Level::INFO => EnvFilter::new("ripfuzz=info,revm=error"),
            tracing::Level::DEBUG => EnvFilter::new("ripfuzz=debug,revm=warn"),
            tracing::Level::TRACE => EnvFilter::new("trace"),
        };

        // 4. Install the subscriber.
        // 4a. Quiet: file only, so `--quiet` and tests cannot leak to the
        //     terminal.
        if self.quiet {
            let Some(file) = file else {
                return Ok(());
            };
            let file_layer = tracing_fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(file))
                .with_span_events(FmtSpan::CLOSE);
            let _ = tracing_subscriber::registry()
                .with(file_layer.with_filter(filter))
                .try_init();
            return Ok(());
        }

        // 4b. Default: compact stderr plus the full file when enabled.
        //     Cast to a fn pointer: `FormatTime` covers
        //     `fn(&mut Writer<'_>) -> fmt::Result`, not the zero-sized fn item
        //     type.
        let stderr_layer = tracing_fmt::layer()
            .with_timer(simple_time as fn(&mut Writer<'_>) -> fmt::Result)
            .with_target(false)
            .with_ansi(std::io::stderr().is_terminal())
            .fmt_fields(ConsoleFields)
            .with_writer(std::io::stderr);
        match file {
            Some(file) => {
                let file_layer = tracing_fmt::layer()
                    .with_ansi(false)
                    .with_writer(Mutex::new(file))
                    .with_span_events(FmtSpan::CLOSE);
                let _ = tracing_subscriber::registry()
                    .with(stderr_layer.with_filter(filter.clone()))
                    .with(file_layer.with_filter(filter))
                    .try_init();
            }
            None => {
                let _ = tracing_subscriber::registry()
                    .with(stderr_layer.with_filter(filter))
                    .try_init();
            }
        }

        Ok(())
    }
}

/// Create a log file, making its parent directories first.
fn open_log_file(path: impl AsRef<Path>) -> Result<File> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    File::create(path).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Arc;
    use std::sync::Mutex;

    use tracing::warn;
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    #[derive(Clone)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn emit_retry_warning() {
        let url = "https://eth-mainnet.g.alchemy.com/v2/secret-key";
        let request = r#"[{"jsonrpc":"2.0","id":0,"method":"eth_getBalance","params":["0xabc"]}]"#;
        let error = concat!(
            r#"RPC error 429: JSON-RPC response contains error object: "#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":429,"message":"rate limited"}}"#,
        );
        warn!(
            retry = 1,
            retries = 3,
            backoff_ms = 100,
            items = 18,
            url = %url,
            request = %request,
            error = %error,
            "transient RPC error; retrying batch"
        );
    }

    fn format_console() -> String {
        let buf = Buffer(Arc::new(Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_target(false)
            .without_time()
            .fmt_fields(ConsoleFields)
            .finish();
        tracing::subscriber::with_default(subscriber, emit_retry_warning);
        String::from_utf8(buf.0.lock().unwrap().clone()).expect("log output must be utf-8")
    }

    fn format_file() -> String {
        let buf = Buffer(Arc::new(Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_target(false)
            .without_time()
            .finish();
        tracing::subscriber::with_default(subscriber, emit_retry_warning);
        String::from_utf8(buf.0.lock().unwrap().clone()).expect("log output must be utf-8")
    }

    /// Terminal retry warnings must stay one line: no JSON payload, no API-key
    /// URL path, and a short error prefix.
    #[test]
    fn console_retry_warning_is_one_line() {
        let out = format_console();
        assert_eq!(
            out,
            " WARN transient RPC error; retrying batch retry=1 retries=3 backoff_ms=100 items=18 url=https://eth-mainnet.g.alchemy.com error=RPC error 429\n"
        );
    }

    /// The campaign log file still records the full URL, request payload, and
    /// error body so a noisy retry can be diagnosed after the run.
    #[test]
    fn campaign_log_keeps_request_payload_and_full_url() {
        let out = format_file();
        assert_eq!(
            out,
            " WARN transient RPC error; retrying batch retry=1 retries=3 backoff_ms=100 items=18 url=https://eth-mainnet.g.alchemy.com/v2/secret-key request=[{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"eth_getBalance\",\"params\":[\"0xabc\"]}] error=RPC error 429: JSON-RPC response contains error object: {\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":429,\"message\":\"rate limited\"}}\n"
        );
    }

    /// `disable_log_file` must not create `.ripfuzz` state, so commands like
    /// `init` can report errors without touching a fresh project. Quiet mode
    /// keeps the test from installing a global subscriber.
    #[test]
    fn disable_log_file_creates_no_ripfuzz_directory() {
        let dir = tempfile::tempdir().unwrap();
        Logger::new()
            .with_root(dir.path())
            .with_quiet(true)
            .disable_log_file()
            .init()
            .unwrap();
        assert!(!dir.path().join(".ripfuzz").exists());
    }
}
