//! Solidity compiler management for `ripfuzz`.
//!
//! Handles downloading and verifying `solc` static binaries from
//! `https://binaries.soliditylang.org` and exposing a builder API for
//! compilation.
//!
//! ```rust
//! use ripfuzz::solc::Solc;
//!
//! let solc = Solc::new().with_version("0.8.28").with_target("src/MyHarness.sol");
//! // solc.compile()?;
//! ```

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};
use solc::{OutputSelector, StandardJSONInput, StandardJSONOutput};
use tracing::info;

pub use installer::SolcInstaller;

pub mod installer;

/// Solidity compiler builder.
#[derive(Clone, Debug, Default)]
pub struct Solc {
    version: Option<String>,
    target: Option<PathBuf>,
    out: Option<PathBuf>,
}

impl Solc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_target(mut self, target: impl AsRef<Path>) -> Self {
        self.target = Some(target.as_ref().to_path_buf());
        self
    }

    pub fn with_out(mut self, out: impl AsRef<Path>) -> Self {
        self.out = Some(out.as_ref().to_path_buf());
        self
    }

    pub fn out_dir(&self) -> PathBuf {
        self.out
            .clone()
            .unwrap_or_else(|| PathBuf::from(".ripfuzz/out"))
    }

    pub fn compile(self) -> Result<()> {
        let version = self
            .version
            .as_deref()
            .context("solc version not set, call Solc::new().with_version(..)")?;
        let target = self
            .target
            .as_deref()
            .context("solc target not set, call Solc::new().with_target(..)")?;

        ensure!(
            target.is_file(),
            "harness file `{}` not found",
            target.display()
        );

        let out_dir = self.out_dir();
        let installer = SolcInstaller::new(version);
        installer.ensure_installed()?;

        info!(
            version = %version,
            target = %target.display(),
            out = %out_dir.display(),
            "compiling harness"
        );

        let sources = collect_sources(target)?;
        let input = build_input(sources);
        let output = run_solc(version, &input)?;
        write_output(&out_dir, &output)?;

        info!(
            version = %version,
            out = %out_dir.display(),
            "compilation succeeded"
        );

        Ok(())
    }
}

fn strip_block_comments(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut in_block = false;
    while let Some(c) = chars.next() {
        if !in_block && c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            in_block = true;
            out.push(' ');
            out.push(' ');
            continue;
        }
        if in_block && c == '*' && chars.peek() == Some(&'/') {
            chars.next();
            in_block = false;
            out.push(' ');
            out.push(' ');
            continue;
        }
        if in_block {
            if c == '\n' {
                out.push('\n');
            } else {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn extract_imports(content: &str) -> Vec<String> {
    let cleaned = strip_block_comments(content);
    let mut imports = Vec::new();
    for line in cleaned.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        for segment in trimmed.split(';') {
            let seg = segment.trim_start();
            if seg.is_empty() || seg.starts_with("//") {
                continue;
            }
            if !seg.starts_with("import") {
                continue;
            }
            let after = &seg["import".len()..];
            if !after.is_empty()
                && !after.starts_with(char::is_whitespace)
                && !after.starts_with('"')
                && !after.starts_with('\'')
                && !after.starts_with('{')
                && !after.starts_with('*')
            {
                continue;
            }
            let mut chars = seg.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch == '"' || ch == '\'' {
                    let quote = ch;
                    let mut path = String::new();
                    while let Some(&next) = chars.peek() {
                        if next == quote {
                            chars.next();
                            break;
                        }
                        path.push(next);
                        chars.next();
                    }
                    if path.ends_with(".sol") {
                        imports.push(path);
                    }
                }
            }
        }
    }
    imports
}

fn normalize_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let mut components: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                if matches!(
                    components.last(),
                    Some(Component::Normal(_)) | Some(Component::ParentDir)
                ) {
                    components.pop();
                } else if !matches!(components.last(), Some(Component::RootDir)) {
                    components.push(comp);
                }
            }
            Component::CurDir => {}
            _ => components.push(comp),
        }
    }
    let mut out = PathBuf::new();
    for comp in components {
        out.push(comp.as_os_str());
    }
    out
}

fn collect_sources(target: impl AsRef<Path>) -> Result<HashMap<PathBuf, String>> {
    let target = target.as_ref();
    let mut sources = HashMap::new();
    let mut visited = HashSet::new();
    let mut stack = vec![target.to_path_buf()];

    while let Some(path) = stack.pop() {
        let canonical = path
            .canonicalize()
            .unwrap_or_else(|_| normalize_path(&path));
        if visited.contains(&canonical) {
            continue;
        }
        // checkrs: allow(clone_in_loops)
        visited.insert(canonical.clone());
        let content = fs::read_to_string(&canonical)
            .with_context(|| format!("failed to read {}", canonical.display()))?;
        let imports = extract_imports(&content);
        // checkrs: allow(clone_in_loops)
        sources.insert(canonical.clone(), content);
        let parent = canonical
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        for import in imports {
            let import_path = parent.join(&import);
            let normalized = normalize_path(&import_path);
            if normalized.is_file() {
                stack.push(normalized);
                continue;
            }
            if let Ok(canonical_import) = import_path.canonicalize()
                && canonical_import.is_file()
            {
                stack.push(canonical_import);
                continue;
            }
            if Path::new(&import).is_file() {
                stack.push(PathBuf::from(&import));
            }
        }
    }

    Ok(sources)
}

fn build_input(sources: HashMap<PathBuf, String>) -> StandardJSONInput {
    let mut input = StandardJSONInput::new();
    for (path, content) in sources {
        input = input.add_source(path, content);
    }
    input.output_selection(
        vec![
            OutputSelector::Abi,
            OutputSelector::Metadata,
            OutputSelector::StorageLayout,
            OutputSelector::EvmBytecodeObject,
            OutputSelector::EvmBytecodeSourceMap,
            OutputSelector::EvmBytecodeLinkReferences,
            OutputSelector::EvmDeployedBytecodeObject,
            OutputSelector::EvmDeployedBytecodeSourceMap,
            OutputSelector::EvmDeployedBytecodeLinkReferences,
            OutputSelector::EvmDeployedBytecodeImmutableReferences,
            OutputSelector::EvmMethodIdentifiers,
        ],
        vec![OutputSelector::Ast],
    )
}

fn run_solc(version: &str, input: &StandardJSONInput) -> Result<StandardJSONOutput> {
    let installer = SolcInstaller::new(version);
    let binary = installer.binary_path();
    let input_json = serde_json::to_string(input).context("failed to serialize solc input")?;

    let mut child = Command::new(&binary)
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

    let output = child
        .wait_with_output()
        .context("failed to wait for solc")?;
    let stdout = String::from_utf8(output.stdout).context("solc output not utf8")?;
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let parsed: StandardJSONOutput = serde_json::from_str(&stdout)
        .with_context(|| format!("failed to parse solc output: {stdout} stderr: {stderr}"))?;

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
        !(parsed.contracts.is_empty() && parsed.sources.is_empty() && !stderr.trim().is_empty()),
        "solc compilation failed: {}",
        stderr.trim()
    );

    Ok(parsed)
}

fn write_output(out_dir: impl AsRef<Path>, output: &StandardJSONOutput) -> Result<()> {
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir).context("failed to create out dir")?;

    let full_path = out_dir.join("output.json");
    fs::write(
        &full_path,
        serde_json::to_string_pretty(output).context("failed to serialize output")?,
    )
    .with_context(|| format!("failed to write {}", full_path.display()))?;

    for (source_path, contracts) in &output.contracts {
        let file_name = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.sol");
        let dir = out_dir.join(file_name);
        fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
        let source_output = output.sources.get(source_path);
        let ast = source_output.and_then(|s| s.ast.as_ref());
        let id = source_output.map(|s| s.id).unwrap_or(0);
        for (contract_name, contract) in contracts {
            let artifact = serde_json::json!({
                "abi": contract.abi,
                "metadata": contract.metadata,
                "storageLayout": contract.storage_layout,
                "evm": contract.evm,
                "ast": ast,
                "id": id,
            });
            let path = dir.join(format!("{contract_name}.json"));
            fs::write(
                &path,
                serde_json::to_string_pretty(&artifact).context("failed to serialize artifact")?,
            )
            .with_context(|| format!("failed to write {}", path.display()))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_imports_simple() {
        let content = r#"import "./Support.sol"; import {Lib} from "./Lib.sol";"#;
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Support.sol", "./Lib.sol"]);
    }

    #[test]
    fn extract_imports_single_quotes() {
        let content = r"import './Foo.sol';";
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Foo.sol"]);
    }

    #[test]
    fn extract_imports_no_imports() {
        let content = "contract Foo {}";
        let imports = extract_imports(content);
        assert!(imports.is_empty());
    }

    #[test]
    fn extract_imports_ignores_line_comment() {
        let content = "// import \"./Foo.sol\";\nimport \"./Bar.sol\";";
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Bar.sol"]);
    }

    #[test]
    fn extract_imports_ignores_block_comment() {
        let content = "/* import \"./Foo.sol\"; */\nimport \"./Bar.sol\";";
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Bar.sol"]);
    }

    #[test]
    fn extract_imports_ignores_import_with_prefix() {
        let content = "contract Foo {}\n  // import \"./Foo.sol\";\n/*\nimport \"./Bar.sol\";\n*/\nimport \"./Baz.sol\";";
        let imports = extract_imports(content);
        assert_eq!(imports, vec!["./Baz.sol"]);
    }

    #[test]
    fn normalize_path_cleans() {
        let p = PathBuf::from("a/./b/../c.sol");
        assert_eq!(normalize_path(&p), PathBuf::from("a/c.sol"));
    }
}
