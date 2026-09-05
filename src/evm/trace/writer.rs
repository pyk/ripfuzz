//! Saving execution traces as timestamped log files.
//!
//! [`ExecutionTraceWriter`] renders a [`Trace`] through its
//! [`TraceContext`](super::TraceContext) and writes the output under
//! `{root}/.ripfuzz/traces`, so every command persists traces with the same
//! file naming and error reporting.
//!
//! ```rust,no_run
//! use ripfuzz::evm::{ExecutionTraceWriter, Trace, TraceContext};
//!
//! # let trace: Trace = todo!();
//! let writer = ExecutionTraceWriter::new(std::path::Path::new("."))
//!     .with_trace_context(TraceContext::new());
//! let path = writer.write(&trace).unwrap();
//! println!("execution trace: {}", path.display());
//! ```

use std::fs;
use std::path::{Path, PathBuf, absolute};

use anyhow::{Context, Result};

use crate::cli::RunId;
use crate::evm::{Trace, TraceContext};

/// Writes execution traces to `{root}/.ripfuzz/traces`.
///
/// Each trace is saved as `{unix-timestamp}-{id}.log` and the absolute path
/// is returned so logs and errors can point at the file. Commands that mint
/// a run id pass it via `with_run_id` so traces save as `{run}-{id}.log`.
#[derive(Debug, Clone)]
pub struct ExecutionTraceWriter {
    root: PathBuf,
    trace_context: TraceContext,
    run_id: Option<RunId>,
}

impl ExecutionTraceWriter {
    /// Create a writer that saves traces under the project root.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            trace_context: TraceContext::new(),
            run_id: None,
        }
    }

    /// Set the trace context used to format and decode saved traces.
    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = trace_context;
        self
    }

    /// Name traces after the run so artifacts of one campaign share their
    /// filename stem. Without it each trace gets a timestamped id.
    pub fn with_run_id(mut self, run_id: &RunId) -> Self {
        self.run_id = Some(run_id.clone());
        self
    }

    /// Render and save an execution trace, returning its absolute path.
    pub fn write(&self, trace: &Trace) -> Result<PathBuf> {
        // 1. Write the execution trace to a timestamped trace file.
        let trace_dir = self.root.join(".ripfuzz").join("traces");
        fs::create_dir_all(&trace_dir)?;
        let trace_file = trace_dir.join(self.file_name());
        let trace = trace.display_with(&self.trace_context).to_string();
        fs::write(&trace_file, trace)
            .with_context(|| format!("failed to write {}", trace_file.display()))?;

        // 2. Return the absolute path so logs and errors can point at the file.
        Ok(absolute(trace_file)?)
    }

    /// Trace file name for this writer, without the parent directory.
    fn file_name(&self) -> String {
        // 1. Share the run stem when the command minted one.
        if let Some(run) = &self.run_id {
            return format!("{run}-{}.log", trace_id());
        }

        // 2. Fall back to a timestamped id for standalone use.
        let timestamp = jiff::Timestamp::now().as_second();
        format!("{timestamp}-{}.log", trace_id())
    }
}

/// Short unique id for a trace file name.
fn trace_id() -> String {
    let uuid: String = uuid::Uuid::new_v4().into();
    uuid.split('-').next().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_uses_the_run_stem_when_set() {
        let writer = ExecutionTraceWriter::new(Path::new(".")).with_run_id(&RunId::new());
        let (stem, id) = writer
            .file_name()
            .strip_suffix(".log")
            .unwrap()
            .rsplit_once('-')
            .map(|(stem, id)| (stem.to_owned(), id.to_owned()))
            .unwrap();

        assert_eq!(stem, writer.run_id.unwrap().to_string());
        assert_eq!(id.len(), 8);
    }

    #[test]
    fn file_name_falls_back_to_a_timestamped_id() {
        let name = ExecutionTraceWriter::new(Path::new(".")).file_name();
        let (timestamp, id) = name.strip_suffix(".log").unwrap().split_once('-').unwrap();

        assert!(timestamp.parse::<i64>().is_ok());
        assert_eq!(id.len(), 8);
    }
}
