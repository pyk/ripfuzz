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

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use solc::StandardJSONOutput;
use tracing::info;

pub use exec::SolcExecutor;
pub use input::StandardJSONInputBuilder;
pub use installer::SolcInstaller;
pub use remappings::RemappingsResolver;
pub use source::SourceResolver;

pub mod exec;
pub mod input;
pub mod installer;
pub mod remappings;
pub mod source;

/// Solidity compiler builder.
///
/// Paths set via `with_target` and `with_out` may be relative.
///
/// - When a root is set via `with_root`, relative paths resolve against it
/// - Imports are resolved using remappings from `{root}/remappings.txt` when
///   present
#[derive(Clone, Debug, Default)]
pub struct Solc {
    version: Option<String>,
    target: Option<PathBuf>,
    out: Option<PathBuf>,
    root: Option<PathBuf>,
}

impl Solc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_root(mut self, root: impl AsRef<Path>) -> Self {
        self.root = Some(root.as_ref().to_path_buf());
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
        self.resolve(self.out.as_deref().unwrap_or(Path::new(".ripfuzz/out")))
    }

    fn resolve(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(root) = &self.root {
            root.join(path)
        } else {
            path.to_path_buf()
        }
    }

    pub fn compile(self) -> Result<()> {
        // 1. Resolve the configured version and target.
        let version = self
            .version
            .as_deref()
            .context("solc version not set, call Solc::new().with_version(..)")?;
        let target = self
            .target
            .as_deref()
            .context("solc target not set, call Solc::new().with_target(..)")?;
        let target = self.resolve(target);

        // 2. Ensure the harness target exists.
        ensure!(
            target.is_file(),
            "harness file `{}` not found",
            target.display()
        );

        // 3. Ensure the solc binary is installed and log the compile plan.
        let out_dir = self.out_dir();
        let installer = SolcInstaller::new(version);
        installer.ensure_installed()?;

        info!(
            version = %version,
            target = %target.display(),
            out = %out_dir.display(),
            "compiling harness"
        );

        // 4. Resolve the transitive sources and build the solc input.
        let root = self.root.clone().unwrap_or_else(|| PathBuf::from("."));
        let resolver = SourceResolver::new()
            .with_root(&root)
            .with_remappings(RemappingsResolver::load(&root)?);
        let sources = resolver.resolve(target)?;
        let input = StandardJSONInputBuilder::new()
            .with_sources(sources)
            .with_remappings(resolver.solc_remappings())
            .build();

        // 5. Run solc, write the artifacts, and log the result.
        let output = SolcExecutor::new()
            .with_version(version)
            .with_root(&root)
            .with_input(input)
            .exec()?;
        write_output(&out_dir, &output)?;

        info!(
            version = %version,
            out = %out_dir.display(),
            "compilation succeeded"
        );

        Ok(())
    }
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
