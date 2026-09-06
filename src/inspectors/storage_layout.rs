//! Storage layout inspection for ripfuzz projects.
//!
//! [`StorageLayoutInspector`] reads the `storageLayout` of a contract directly
//! from the compilation output and reports every storage variable with its
//! type, slot, offset, and defining contract.
//!
//! ```rust
//! use ripfuzz::config::Config;
//! use ripfuzz::cli::HarnessId;
//! use ripfuzz::inspectors::StorageLayoutInspector;
//!
//! let root = std::path::Path::new(".");
//! let config = Config::new().with_root(root).load("ripfuzz.toml")?;
//! let target = HarnessId::try_from("src/Voter.sol:Voter")?;
//! // let report = StorageLayoutInspector::new(root, config).inspect(&target)?;
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::cli::HarnessId;
use crate::config::Config;
use crate::inspectors::CompiledTarget;

/// Metadata about a single storage variable.
#[derive(Debug, Clone)]
pub struct StorageVariableInfo {
    /// Variable name, e.g. `total`.
    pub name: String,

    /// Human readable type label from the compilation output,
    /// e.g. `uint256` or `mapping(address => uint256)`.
    pub r#type: String,

    /// Storage slot as a decimal string.
    pub slot: String,

    /// Byte offset within the slot.
    pub offset: String,

    /// Number of bytes the type occupies, as a decimal string.
    pub bytes: String,
}

/// The report produced by [`StorageLayoutInspector`].
#[derive(Debug)]
pub struct StorageLayoutOutput {
    /// Name of the inspected contract.
    pub contract_name: String,

    /// Source file of the inspected contract, relative to the project root.
    pub source_file: String,

    /// Storage variables in slot order.
    pub variables: Vec<StorageVariableInfo>,
}

impl std::fmt::Display for StorageLayoutOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 1. Header.
        writeln!(f, "# {} Storage Layout", self.contract_name)?;
        writeln!(f)?;
        writeln!(f, "- Contract: `{}`", self.contract_name)?;
        writeln!(f, "- File: `{}`", self.source_file)?;
        writeln!(f)?;

        // 2. Summary.
        writeln!(f, "## Summary")?;
        writeln!(f)?;
        writeln!(f, "- {} storage variables", self.variables.len())?;
        writeln!(f)?;

        // 3. Table. An empty layout shows the summary only.
        if self.variables.is_empty() {
            return Ok(());
        }
        let rows: Vec<Vec<String>> = self
            .variables
            .iter()
            .enumerate()
            .map(|(index, variable)| {
                vec![
                    (index + 1).to_string(),
                    format!("`{}`", variable.name),
                    format!("`{}`", variable.r#type),
                    variable.slot.clone(),
                    variable.offset.clone(),
                    variable.bytes.clone(),
                ]
            })
            .collect();
        write_table(f, &["#", "Name", "Type", "Slot", "Offset", "Bytes"], &rows)
    }
}

/// Writes a padded markdown table. The first column is right-aligned, the
/// rest left-aligned.
fn write_table(
    f: &mut std::fmt::Formatter<'_>,
    headers: &[&str],
    rows: &[Vec<String>],
) -> std::fmt::Result {
    // 1. Column widths from the header and every row.
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.len());
        }
    }

    // 2. Header row and the alignment row below it. Alignment cells keep at
    //    least two dashes so every markdown renderer accepts them.
    let header: Vec<String> = headers.iter().map(|header| header.to_string()).collect();
    let separator: Vec<String> = (0..headers.len())
        .map(|index| {
            let dashes = "-".repeat(widths[index].saturating_sub(1).max(2));
            if index == 0 {
                format!("{dashes}:")
            } else {
                format!(":{dashes}")
            }
        })
        .collect();
    write_row(f, &header, &widths)?;
    write_row(f, &separator, &widths)?;

    // 3. One data row per variable.
    for row in rows {
        write_row(f, row, &widths)?;
    }
    Ok(())
}

/// Writes one padded table row.
fn write_row(
    f: &mut std::fmt::Formatter<'_>,
    cells: &[String],
    widths: &[usize],
) -> std::fmt::Result {
    write!(f, "|")?;
    for (index, cell) in cells.iter().enumerate() {
        let width = widths[index];
        if index == 0 {
            write!(f, " {cell:>width$} |")?;
        } else {
            write!(f, " {cell:<width$} |")?;
        }
    }
    writeln!(f)
}

/// Inspects the storage layout of a single contract.
///
/// The inspector compiles the target through the shared solc pipeline, so a
/// cached compilation keyed by the standard JSON input hash skips solc
/// entirely on repeated runs.
pub struct StorageLayoutInspector {
    root: PathBuf,
    config: Config,
}

impl StorageLayoutInspector {
    /// Creates an inspector for a project root and its loaded config.
    pub fn new(root: impl AsRef<Path>, config: Config) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            config,
        }
    }

    /// Inspects the storage layout of `target`.
    pub fn inspect(&self, target: &HarnessId) -> Result<StorageLayoutOutput> {
        // 1. Compile the target or reuse a cached compilation.
        let compiled = CompiledTarget::compile(&self.root, &self.config, target)?;

        // 2. Read the storage layout directly from the compilation output.
        let contract_output = compiled.contract(target)?;
        let layout = contract_output.storage_layout.as_ref();
        let types = layout.and_then(|layout| layout.types.as_ref());
        let mut variables = Vec::new();
        if let Some(layout) = layout {
            for entry in &layout.storage {
                let (type_label, bytes) = match types.and_then(|types| types.get(&entry.r#type)) {
                    // checkrs: allow(clone_in_loops)
                    Some(info) => (info.label.clone(), info.number_of_bytes.clone()),
                    None => ("unknown".to_owned(), "unknown".to_owned()),
                };
                variables.push(StorageVariableInfo {
                    // checkrs: allow(clone_in_loops)
                    name: entry.label.clone(),
                    r#type: type_label,
                    // checkrs: allow(clone_in_loops)
                    slot: entry.slot.clone(),
                    offset: entry.offset.to_string(),
                    bytes,
                });
            }
        }

        // 3. Report the variables in slot order.
        Ok(StorageLayoutOutput {
            contract_name: target.name.clone(),
            source_file: compiled.source_path.display().to_string(),
            variables,
        })
    }
}
