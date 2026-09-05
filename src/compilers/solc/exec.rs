//! Solc execution for compilation.
//!
//! Runs the installed solc binary in standard JSON mode. The child process
//! runs with the project root as its working directory, so source keys and
//! remappings stay root-relative.
//!
//! Outputs are cached under a hash of the solc version and the standard JSON
//! input, so identical compilations reuse the cached output without running
//! solc again.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use solc::{StandardJSONInput, StandardJSONOutput};
use tracing::debug;

use crate::compilers::solc::SolcInstaller;

/// Runs the installed solc binary for a compilation.
///
/// When a cache directory is set, outputs are stored as
/// `{cache}/{hash}.json` keyed by [`SolcExecutor::compile_hash`] and reused
/// for identical inputs without spawning solc.
#[derive(Clone, Debug)]
pub struct SolcExecutor {
    version: Option<String>,
    root: Option<PathBuf>,
    input: Option<StandardJSONInput>,
    cache: Option<PathBuf>,
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
            cache: None,
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

    /// Sets the directory for content-addressed compilation cache entries.
    ///
    /// With a cache set, an input that was compiled before under the same
    /// solc version and settings returns the cached output without running
    /// solc again.
    pub fn with_cache(mut self, dir: impl AsRef<Path>) -> Self {
        self.cache = Some(dir.as_ref().to_path_buf());
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

        // 2. Load the cached output when one covers this exact input.
        //
        //    The cache key hashes the solc version and the full standard JSON
        //    input, so any change to sources, settings, or compiler version
        //    produces a new key and recompiles.
        let cache_path = match &self.cache {
            Some(dir) => Some(dir.join(format!("{}.json", compile_hash(version, &input)?))),
            None => None,
        };
        if let Some(path) = &cache_path
            && let Some(cached) = read_cache(path)
        {
            debug!("using cached solc output {}", path.display());
            return Ok(cached);
        }

        // 3. Ensure the solc binary is installed.
        let installer = SolcInstaller::new(version);
        installer.ensure_installed()?;

        // 4. Resolve the installed binary path.
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

        // 5. Serialize the standard JSON input.
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

        // 6. Spawn solc from the project root and feed it the input.
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

        // 7. Wait for solc and parse its output.
        let output = child
            .wait_with_output()
            .context("failed to wait for solc")?;
        let stdout = String::from_utf8(output.stdout).context("solc output not utf8")?;
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let parsed: StandardJSONOutput = serde_json::from_str(&stdout)
            .with_context(|| format!("failed to parse solc output: {stdout} stderr: {stderr}"))?;

        // 8. Fail when the compiler reported errors.
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

        // 9. Persist the raw output for future runs with the same input.
        if let Some(path) = &cache_path {
            write_cache(path, &stdout)?;
        }

        Ok(parsed)
    }
}

/// Content hash of a compilation.
///
/// Hashes the solc version and a canonical serialization of the standard
/// JSON input. Sources sort by path because the input stores them in a map
/// with nondeterministic iteration order, so identical inputs always hash
/// to the same key.
fn compile_hash(version: &str, input: &StandardJSONInput) -> Result<String> {
    // 1. Serialize every source sorted by path.
    let mut sources = BTreeMap::new();
    for (path, source) in &input.sources {
        let json = serde_json::to_value(source).context("failed to serialize solc source")?;
        sources.insert(path.to_string_lossy().to_string(), json);
    }

    // 2. Hash the version together with the canonical input. Object keys
    //    sort recursively because some settings fields are maps whose
    //    iteration order changes between runs.
    let canonical = canonicalize_keys(&serde_json::json!({
        "version": version,
        "language": input.language,
        "sources": sources,
        "settings": input.settings,
    }));
    let bytes = serde_json::to_vec(&canonical).context("failed to serialize solc input")?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

/// Rebuilds a JSON value with every object's keys sorted, so values that
/// pass through hash maps serialize identically across runs.
fn canonicalize_keys(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            // checkrs: allow(clone_in_loops)
            for key in keys {
                // checkrs: allow(clone_in_loops)
                sorted.insert(key.clone(), canonicalize_keys(&map[key]));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalize_keys).collect())
        }
        other => other.clone(),
    }
}

/// Loads a cached compilation output, treating an unreadable cache entry as
/// a miss so the next run recompiles and rewrites it.
fn read_cache(path: impl AsRef<Path>) -> Option<StandardJSONOutput> {
    let path = path.as_ref();
    let content = fs::read_to_string(path).ok()?;
    match serde_json::from_str(&content) {
        Ok(output) => Some(output),
        Err(e) => {
            debug!("ignoring unreadable solc cache {}: {e}", path.display());
            None
        }
    }
}

/// Persists a cache entry atomically, writing to a temporary file and
/// renaming it so concurrent runs never observe a partial output.
fn write_cache(path: impl AsRef<Path>, output: &str) -> Result<()> {
    let path = path.as_ref();

    // 1. Ensure the cache directory exists.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    // 2. Write the temporary file and swap it into place. The temporary
    //    name is unique per writer, so concurrent compilations of the same
    //    input never rename the same temporary file twice.
    let tmp = path.with_extension(format!("json.{}.tmp", fastrand::u64(..)));
    fs::write(&tmp, output).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use solc::Optimizer;

    use super::*;

    #[test]
    fn compile_hash_is_stable_across_source_insertion_order() {
        let first = StandardJSONInput::new()
            .add_source("A.sol", "contract A {}")
            .add_source("B.sol", "contract B {}");
        let second = StandardJSONInput::new()
            .add_source("B.sol", "contract B {}")
            .add_source("A.sol", "contract A {}");

        assert_eq!(
            compile_hash("0.8.36", &first).unwrap(),
            compile_hash("0.8.36", &second).unwrap()
        );
    }

    #[test]
    fn compile_hash_changes_with_source_content() {
        let first = StandardJSONInput::new().add_source("A.sol", "contract A {}");
        let second = StandardJSONInput::new().add_source("A.sol", "contract A { uint256 x; }");

        assert_ne!(
            compile_hash("0.8.36", &first).unwrap(),
            compile_hash("0.8.36", &second).unwrap()
        );
    }

    #[test]
    fn compile_hash_changes_with_version_and_settings() {
        let plain = StandardJSONInput::new().add_source("A.sol", "contract A {}");
        let mut optimized = StandardJSONInput::new().add_source("A.sol", "contract A {}");
        optimized.settings.optimizer = Some(Optimizer {
            enabled: Some(true),
            runs: Some(200),
            details: None,
        });

        assert_ne!(
            compile_hash("0.8.36", &plain).unwrap(),
            compile_hash("0.8.40", &plain).unwrap()
        );
        assert_ne!(
            compile_hash("0.8.36", &plain).unwrap(),
            compile_hash("0.8.36", &optimized).unwrap()
        );
    }

    #[test]
    fn canonicalize_keys_sorts_object_keys_recursively() {
        let one = serde_json::json!({"b": {"y": 1, "x": 2}, "a": 3});
        let two = serde_json::json!({"a": 3, "b": {"x": 2, "y": 1}});

        assert_eq!(canonicalize_keys(&one), canonicalize_keys(&two));
        assert_eq!(
            serde_json::to_vec(&canonicalize_keys(&one)).unwrap(),
            serde_json::to_vec(&canonicalize_keys(&two)).unwrap()
        );
    }
}
