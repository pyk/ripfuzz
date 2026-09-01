//! Solc execution for compilation.
//!
//! Runs the installed solc binary in standard JSON mode. The child process
//! runs with the project root as its working directory, so source keys and
//! remappings stay root-relative.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};
use solc::{StandardJSONInput, StandardJSONOutput};

use crate::compilers::solc::SolcInstaller;

/// Runs the installed solc binary for a compilation.
#[derive(Clone, Debug)]
pub struct SolcExecutor {
    version: Option<String>,
    root: Option<PathBuf>,
    input: Option<StandardJSONInput>,
}

impl Default for SolcExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl SolcExecutor {
    pub fn new() -> Self {
        Self {
            version: None,
            root: None,
            input: None,
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_root(mut self, root: impl AsRef<Path>) -> Self {
        self.root = Some(root.as_ref().to_path_buf());
        self
    }

    pub fn with_input(mut self, input: StandardJSONInput) -> Self {
        self.input = Some(input);
        self
    }

    /// Runs solc against the standard JSON input.
    pub fn exec(self) -> Result<StandardJSONOutput> {
        // 1. Resolve the configured version, root, and input.
        let input = self
            .input
            .context("solc input not set, call SolcExecutor::new().with_input(..)")?;
        let root = self.root.unwrap_or_else(|| PathBuf::from("."));
        let version = self
            .version
            .as_deref()
            .context("solc version not set, call SolcExecutor::new().with_version(..)")?;

        // 2. Resolve the installed binary path.
        let installer = SolcInstaller::new(version);
        let binary = installer.binary_path();
        // The binary lives relative to the process cwd, so resolve it before
        // switching the child into the project root.
        let binary = if binary.is_absolute() {
            binary
        } else {
            std::env::current_dir()
                .context("failed to get current dir")?
                .join(binary)
        };

        // 3. Serialize the standard JSON input.
        //
        //    The solc crate serializes `via_ir` as `viaIr`, but solc expects
        //    the `viaIR` key, so rename it before feeding the input to the
        //    compiler.
        let mut input_value =
            serde_json::to_value(&input).context("failed to serialize solc input")?;
        if let Some(settings) = input_value
            .get_mut("settings")
            .and_then(|settings| settings.as_object_mut())
            && let Some(via_ir) = settings.remove("viaIr")
        {
            settings.insert("viaIR".to_owned(), via_ir);
        }
        let input_json =
            serde_json::to_string(&input_value).context("failed to serialize solc input")?;

        // 4. Spawn solc from the project root and feed it the input.
        let mut child = Command::new(&binary)
            .current_dir(&root)
            .arg("--standard-json")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn solc {}", binary.display()))?;

        child
            .stdin
            .take()
            .context("failed to open solc stdin")?
            .write_all(input_json.as_bytes())
            .context("failed to write solc input")?;

        // 5. Wait for solc and parse its output.
        let output = child
            .wait_with_output()
            .context("failed to wait for solc")?;
        let stdout = String::from_utf8(output.stdout).context("solc output not utf8")?;
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let parsed: StandardJSONOutput = serde_json::from_str(&stdout)
            .with_context(|| format!("failed to parse solc output: {stdout} stderr: {stderr}"))?;

        // 6. Fail when the compiler reported errors.
        let error_msgs: Vec<String> = parsed.errors.as_ref().map_or(Vec::new(), |errors| {
            errors
                .iter()
                .filter_map(|err| {
                    let severity_str = serde_json::to_value(&err.severity)
                        .ok()
                        .and_then(|v| v.as_str().map(ToOwned::to_owned))?;
                    if severity_str != "error" {
                        return None;
                    }
                    Some(
                        err.formatted_message
                            .clone()
                            .unwrap_or_else(|| err.message.clone()),
                    )
                })
                .collect()
        });

        ensure!(
            error_msgs.is_empty(),
            "solc compilation failed:\n{}",
            error_msgs.join("\n")
        );

        ensure!(
            !(parsed.contracts.is_empty()
                && parsed.sources.is_empty()
                && !stderr.trim().is_empty()),
            "solc compilation failed: {}",
            stderr.trim()
        );

        Ok(parsed)
    }
}
