//! Foundry `forge` command wrappers for compilation.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Result, bail};

/// Run `forge build <contract_path>` inside the given project root.
pub fn build(project_root: impl AsRef<Path>, contract_path: impl AsRef<Path>) -> Result<()> {
    let output = Command::new("forge")
        .arg("build")
        .arg(contract_path.as_ref())
        .current_dir(project_root.as_ref())
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{}", stderr.trim())
    }
}

/// Return the relative path of the *latest* build-info file inside
/// `<out>/build-info/`.
#[allow(clippy::result_map_or_into_option)]
pub fn latest_build_info(out_dir: impl AsRef<Path>) -> Result<Option<String>> {
    let out_dir = out_dir.as_ref();
    let build_info = out_dir.join("build-info");
    if !build_info.exists() {
        return Ok(None);
    }

    let mut entries: Vec<String> = fs::read_dir(build_info)?
        .filter_map(|e| e.map_or(None, Some))
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    entries.sort(); // lexicographic sort; build-info names are timestamps
    Ok(entries.pop())
}

/// List the names of compiled artifacts (`.json` files) inside
/// `<out>/<contract>.sol/`.
#[allow(clippy::result_map_or_into_option)]
pub fn list_artifacts(out_dir: impl AsRef<Path>, contract_name: &str) -> Result<Vec<String>> {
    let out_dir = out_dir.as_ref();
    let dir = out_dir.join(format!("{contract_name}.sol"));
    if !dir.exists() {
        return Ok(vec![]);
    }

    Ok(fs::read_dir(dir)?
        .filter_map(|e| e.map_or(None, Some))
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect())
}
