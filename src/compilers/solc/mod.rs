//! Solidity compiler management for `ripfuzz`.
//!
//! Handles downloading and verifying `solc` static binaries from
//! `https://binaries.soliditylang.org` and exposing a builder API for
//! compilation.
//!
//! ```rust
//! use ripfuzz::compilers::solc::Solc;
//!
//! let solc = Solc::new().with_version("0.8.28").with_target("src/MyHarness.sol");
//! // let solc_output = solc.compile()?;
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use solc::{ContractOutput, EvmVersion, StandardJSONOutput};
use tracing::info;

pub use exec::SolcExecutor;
pub use input::StandardJSONInputBuilder;
pub use installer::SolcInstaller;
pub use remappings::RemappingsResolver;
pub use source::SourceResolver;

use crate::harness::HarnessId;

pub mod exec;
pub mod input;
pub mod installer;
pub mod remappings;
pub mod source;

/// Result of a successful `Solc::compile` call.
///
/// Carries the identifier of the compiled target contract next to the raw
/// solc output, so consumers can extract the target contract and render
/// source-aware traces without re-resolving paths.
#[derive(Debug, Clone)]
pub struct SolcOutput {
    /// Identifier of the compiled target contract (source path and name).
    pub id: HarnessId,
    /// Raw solc standard JSON output for the whole compilation unit.
    pub output: StandardJSONOutput,
}

impl SolcOutput {
    /// The compiled target contract.
    pub fn contract(&self) -> Result<&ContractOutput> {
        let contracts = self.output.contracts.get(&self.id.path).with_context(|| {
            format!(
                "harness source `{}` not found in compilation output",
                self.id.path.display()
            )
        })?;
        contracts.get(&self.id.name).with_context(|| {
            let mut names: Vec<String> = contracts.keys().map(|name| name.to_owned()).collect();
            names.sort();
            format!(
                "contract `{}` not found in `{}`, available contracts: {}",
                self.id.name,
                self.id.path.display(),
                names.join(", ")
            )
        })
    }

    /// Hex-encoded initcode of the target contract.
    pub fn initcode(&self) -> Result<&str> {
        let initcode = self
            .contract()?
            .evm
            .as_ref()
            .and_then(|evm| evm.bytecode.as_ref())
            .and_then(|bytecode| bytecode.object.as_ref())
            .context("harness initcode missing from compilation output")?;
        ensure!(
            !initcode.is_empty(),
            "harness contract `{}` has empty initcode",
            self.id
        );
        Ok(initcode)
    }
}

/// Solidity compiler builder.
///
/// Paths set via `with_target` and `with_out` may be relative.
///
/// - When a root is set via `with_root`, relative paths resolve against it
/// - Imports are resolved using remappings from `{root}/remappings.txt` when
///   present, with remappings set via `with_remappings` taking precedence
///   over `remappings.txt` entries with the same prefix
/// - The harness contract name defaults to the target file stem and can be
///   overridden with `with_name`
/// - Artifacts are written under a namespace derived from the target source
///   path, so targets sharing an out dir never overwrite each other
#[derive(Clone, Debug, Default)]
pub struct Solc {
    version: Option<String>,
    target: Option<PathBuf>,
    name: Option<String>,
    out: Option<PathBuf>,
    root: Option<PathBuf>,
    evm_version: Option<EvmVersion>,
    optimizer: Option<(bool, usize)>,
    via_ir: Option<bool>,
    remappings: Vec<String>,
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

    /// Set the harness contract name within the target file.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_out(mut self, out: impl AsRef<Path>) -> Self {
        self.out = Some(out.as_ref().to_path_buf());
        self
    }

    /// Set the target EVM version for code generation.
    pub fn with_evm_version(mut self, evm_version: EvmVersion) -> Self {
        self.evm_version = Some(evm_version);
        self
    }

    /// Enable the optimizer and set the number of runs.
    pub fn with_optimizer(mut self, enabled: bool, runs: usize) -> Self {
        self.optimizer = Some((enabled, runs));
        self
    }

    /// Enable the IR-based compilation pipeline.
    pub fn with_via_ir(mut self, via_ir: bool) -> Self {
        self.via_ir = Some(via_ir);
        self
    }

    /// Set `prefix=target` remappings. They take precedence over
    /// `remappings.txt` entries with the same prefix.
    pub fn with_remappings(mut self, remappings: Vec<String>) -> Self {
        self.remappings = remappings;
        self
    }

    pub fn out_dir(&self) -> PathBuf {
        self.resolve(self.out.as_deref().unwrap_or(Path::new(".ripfuzz/solc")))
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

    pub fn compile(self) -> Result<SolcOutput> {
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

        // 3. Log the compile plan. The executor installs the binary and runs
        //    solc only when no cached output covers the compilation.
        let out_dir = self.out_dir();
        info!("compiling harness {}", strip_dot_prefix(&target));

        // 4. Resolve the transitive sources and build the solc input.
        let root = self.root.clone().unwrap_or_else(|| PathBuf::from("."));
        let remappings =
            RemappingsResolver::load(&root)?.with_remappings(self.remappings.clone())?;
        let resolver = SourceResolver::new()
            .with_root(&root)
            .with_remappings(remappings);
        let sources = resolver.resolve(&target)?;
        let mut input = StandardJSONInputBuilder::new()
            .with_sources(sources)
            .with_remappings(resolver.solc_remappings());
        if let Some(evm_version) = self.evm_version {
            input = input.with_evm_version(evm_version);
        }
        if let Some((enabled, runs)) = self.optimizer {
            input = input.with_optimizer(enabled, runs);
        }
        if let Some(via_ir) = self.via_ir {
            input = input.with_via_ir(via_ir);
        }
        let input = input.build();

        // 5. Run solc or reuse a cached compilation.
        let output = SolcExecutor::new()
            .with_version(version)
            .with_root(&root)
            .with_input(input)
            .with_cache(&out_dir)
            .exec()?;

        // 6. Resolve the target path relative to the root. The solc output
        //    keys sources by this path, so it namespaces the written
        //    artifacts and identifies the harness contract.
        let canonical_target = target.canonicalize().unwrap_or_else(|_| target.clone());
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
        let source_path = canonical_target
            .strip_prefix(&canonical_root)
            .unwrap_or(&canonical_target)
            .to_path_buf();

        // 7. Write the artifacts under the target namespace and log the
        //    result.
        write_output(&out_dir, &source_path, &output)?;

        info!("compilation succeeded");

        // 8. Identify the compiled target contract.
        let name = match self.name {
            Some(name) => name,
            None => target
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_owned(),
        };
        let id = HarnessId::try_from(format!("{}:{}", source_path.display(), name))?;

        Ok(SolcOutput { id, output })
    }
}

fn write_output(
    out_dir: impl AsRef<Path>,
    namespace: impl AsRef<Path>,
    output: &StandardJSONOutput,
) -> Result<()> {
    let out_dir = out_dir.as_ref().join(namespace.as_ref());
    fs::create_dir_all(&out_dir).context("failed to create out dir")?;

    let full_path = out_dir.join("out.json");
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

fn strip_dot_prefix(path: impl AsRef<Path>) -> String {
    let mut display = path.as_ref().display().to_string();
    loop {
        if let Some(stripped) = display.strip_prefix("./") {
            display = stripped.to_owned();
        } else if let Some(stripped) = display.strip_prefix(".\\") {
            display = stripped.to_owned();
        } else {
            break;
        }
    }
    display
}
