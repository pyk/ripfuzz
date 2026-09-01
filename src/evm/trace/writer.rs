//! Saving execution traces as timestamped log files.
//!
//! [`ExecutionTraceWriter`] renders a [`Trace`] through its
//! [`TraceContext`](super::TraceContext) and writes the output under
//! `{root}/.ripfuzz/traces`, so every command persists traces with the same
//! file naming and error reporting.
//!
//! ```rust,no_run
//! use ripfuzz::{ExecutionTraceWriter, Trace, TraceContext};
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

use crate::evm::{Trace, TraceContext};

/// Writes execution traces to `{root}/.ripfuzz/traces`.
///
/// Each trace is saved as `{unix-timestamp}-{id}.log` and the absolute path
/// is returned so logs and errors can point at the file.
#[derive(Debug, Clone)]
pub struct ExecutionTraceWriter {
    root: PathBuf,
    trace_context: TraceContext,
}

impl ExecutionTraceWriter {
    /// Create a writer that saves traces under the project root.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            trace_context: TraceContext::new(),
        }
    }

    /// Set the trace context used to format and decode saved traces.
    pub fn with_trace_context(mut self, trace_context: TraceContext) -> Self {
        self.trace_context = trace_context;
        self
    }

    /// Render and save an execution trace, returning its absolute path.
    pub fn write(&self, trace: &Trace) -> Result<PathBuf> {
        // 1. Write the execution trace to a timestamped trace file.
        let trace_dir = self.root.join(".ripfuzz").join("traces");
        fs::create_dir_all(&trace_dir)?;
        let timestamp = jiff::Timestamp::now().as_second();
        let trace_file = trace_dir.join(format!("{timestamp}-{}.log", trace_id()));
        let trace = trace.display_with(&self.trace_context).to_string();
        fs::write(&trace_file, trace)
            .with_context(|| format!("failed to write {}", trace_file.display()))?;

        // 2. Return the absolute path so logs and errors can point at the file.
        Ok(absolute(trace_file)?)
    }
}

/// Short unique id for a trace file name.
fn trace_id() -> String {
    let uuid: String = uuid::Uuid::new_v4().into();
    uuid.split('-').next().unwrap_or_default().to_owned()
}
