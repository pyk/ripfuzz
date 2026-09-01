//! Saving coverage reports as `lcov.info` files.
//!
//! [`CoverageWriter`] renders a [`CoverageReport`] and writes it under
//! `{root}/.ripfuzz/coverage`, so every command persists coverage with the
//! same file naming and error reporting.
//!
//! ```rust,no_run
//! use ripfuzz::{CoverageReport, CoverageWriter};
//!
//! # let report: CoverageReport = todo!();
//! let writer = CoverageWriter::new(std::path::Path::new("."));
//! let path = writer.write(&report).unwrap();
//! println!("coverage report: {}", path.display());
//! ```

use std::fs;
use std::path::{Path, PathBuf, absolute};

use anyhow::{Context, Result};

use crate::evm::CoverageReport;

/// Writes coverage reports to `{root}/.ripfuzz/coverage/lcov.info`.
#[derive(Debug, Clone)]
pub struct CoverageWriter {
    root: PathBuf,
}

impl CoverageWriter {
    /// Create a writer that saves coverage under the project root.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Render and save a coverage report, returning its absolute path.
    pub fn write(&self, report: &CoverageReport) -> Result<PathBuf> {
        // 1. Write the report to `.ripfuzz/coverage/lcov.info`.
        let coverage_dir = self.root.join(".ripfuzz").join("coverage");
        fs::create_dir_all(&coverage_dir)?;
        let coverage_file = coverage_dir.join("lcov.info");
        fs::write(&coverage_file, report.to_string())
            .with_context(|| format!("failed to write {}", coverage_file.display()))?;

        // 2. Return the absolute path so logs and errors can point at the file.
        Ok(absolute(coverage_file)?)
    }
}
