//! Saving coverage reports as per-run `.info` files.
//!
//! [`CoverageWriter`] renders a [`CoverageReport`] and writes it under
//! `{root}/.ripfuzz/coverage`, so every command persists coverage with the
//! same file naming and error reporting.
//!
//! ```rust,no_run
//! use ripfuzz::evm::{CoverageReport, CoverageWriter};
//!
//! # let report: CoverageReport = todo!();
//! let writer = CoverageWriter::new(std::path::Path::new("."));
//! let path = writer.write(&report).unwrap();
//! println!("coverage report: {}", path.display());
//! ```

use std::fs;
use std::path::{Path, PathBuf, absolute};

use anyhow::{Context, Result};

use crate::cli::RunId;
use crate::evm::CoverageReport;

/// Writes coverage reports to `{root}/.ripfuzz/coverage`.
///
/// The report is saved as `{run}.info` when the command minted a run id and
/// as `lcov.info` otherwise. The absolute path is returned so logs and
/// errors can point at the file.
#[derive(Debug, Clone)]
pub struct CoverageWriter {
    root: PathBuf,
    run_id: Option<RunId>,
}

impl CoverageWriter {
    /// Create a writer that saves coverage under the project root.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            run_id: None,
        }
    }

    /// Name the report after the run so artifacts of one campaign share
    /// their filename stem. Without it the report overwrites `lcov.info`.
    pub fn with_run_id(mut self, run_id: &RunId) -> Self {
        self.run_id = Some(run_id.clone());
        self
    }

    /// Render and save a coverage report, returning its absolute path.
    pub fn write(&self, report: &CoverageReport) -> Result<PathBuf> {
        // 1. Resolve the report file name for this writer.
        let coverage_dir = self.root.join(".ripfuzz").join("coverage");
        fs::create_dir_all(&coverage_dir)?;
        let coverage_file = coverage_dir.join(self.file_name());
        fs::write(&coverage_file, report.to_string())
            .with_context(|| format!("failed to write {}", coverage_file.display()))?;

        // 2. Return the absolute path so logs and errors can point at the file.
        Ok(absolute(coverage_file)?)
    }

    /// Coverage file name for this writer, without the parent directory.
    fn file_name(&self) -> String {
        // 1. Share the run stem when the command minted one.
        if let Some(run) = &self.run_id {
            return format!("{run}.info");
        }

        // 2. Fall back to the shared report name for standalone use.
        String::from("lcov.info")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_uses_the_run_stem_when_set() {
        let writer = CoverageWriter::new(Path::new(".")).with_run_id(&RunId::new());

        assert!(writer.file_name().ends_with(".info"));
        assert_eq!(
            writer.file_name(),
            format!("{}.info", writer.run_id.unwrap().stem())
        );
    }

    #[test]
    fn file_name_falls_back_to_lcov_info() {
        let writer = CoverageWriter::new(Path::new("."));

        assert_eq!(writer.file_name(), "lcov.info");
    }
}
