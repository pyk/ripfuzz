//! Compilation pipeline shared by contract inspectors.
//!
//! [`CompiledTarget`] compiles a harness target through the shared solc
//! pipeline, so a cached compilation keyed by the standard JSON input hash
//! skips solc entirely on repeated runs.
//!
//! ```rust
//! use ripfuzz::config::Config;
//! use ripfuzz::harness::HarnessId;
//! use ripfuzz::inspectors::CompiledTarget;
//!
//! let root = std::path::Path::new(".");
//! let config = Config::new().with_root(root).load("ripfuzz.toml")?;
//! let target = HarnessId::try_from("src/Voter.sol:Voter")?;
//! // let compiled = CompiledTarget::compile(root, &config, &target)?;
//! # Ok::<(), anyhow::Error>(())
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use solc::{ContractOutput, StandardJSONOutput};

use crate::compilers::solc::{
    RemappingsResolver, SolcExecutor, SourceResolver, StandardJSONInputBuilder,
};
use crate::config::Config;
use crate::harness::HarnessId;

/// The compiled output of a harness target.
pub struct CompiledTarget {
    /// Standard JSON compilation output with every AST.
    pub output: StandardJSONOutput,

    /// Contents of every compiled source, keyed relative to the project root.
    pub sources: HashMap<PathBuf, String>,

    /// Source path of the target, relative to the project root.
    pub source_path: PathBuf,
}

impl CompiledTarget {
    /// Compiles `target` or reuses a cached compilation.
    pub fn compile(root: &Path, config: &Config, target: &HarnessId) -> Result<Self> {
        // 1. Resolve the target source path relative to the root.
        let target_path = root.join(&target.path);
        ensure!(
            target_path.is_file(),
            "contract file `{}` not found",
            target.path.display()
        );
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let canonical_target = target_path
            .canonicalize()
            .unwrap_or_else(|_| target_path.clone());
        let source_path = canonical_target
            .strip_prefix(&canonical_root)
            .unwrap_or(&canonical_target)
            .to_path_buf();

        // 2. Resolve remappings and the transitive sources of the target.
        let remappings =
            RemappingsResolver::load(root)?.with_remappings(config.compile_remappings())?;
        let sources = SourceResolver::new()
            .with_root(root)
            .with_remappings(remappings.clone())
            .resolve(&target_path)?;

        // 3. Build the standard JSON input with the project compile
        //    settings, so the cache key matches a direct compilation of the
        //    same target.
        let mut input = StandardJSONInputBuilder::new()
            .with_sources(sources.clone())
            .with_remappings(remappings.solc_remappings())
            .with_evm_version(config.solc.evm_version.clone())
            .with_optimizer(config.solc.optimizer, config.solc.optimizer_runs);
        if config.solc.via_ir {
            input = input.with_via_ir(true);
        }
        let input = input.build();

        // 4. Run solc or reuse a cached compilation.
        let out_dir = resolve_out_dir(root, config);
        let output = SolcExecutor::new()
            .with_version(&config.solc.version)
            .with_root(root)
            .with_input(input)
            .with_cache(out_dir)
            .exec()?;

        Ok(Self {
            output,
            sources,
            source_path,
        })
    }

    /// Contract output of the target contract.
    pub fn contract(&self, target: &HarnessId) -> Result<&ContractOutput> {
        let contract = self
            .output
            .contracts
            .get(&self.source_path)
            .with_context(|| {
                format!(
                    "contract file `{}` not found in compilation output",
                    self.source_path.display()
                )
            })?;
        contract.get(&target.name).with_context(|| {
            let mut names: Vec<String> = contract.keys().map(|name| name.to_owned()).collect();
            names.sort();
            format!(
                "contract `{}` not found in `{}`, available contracts: {}",
                target.name,
                self.source_path.display(),
                names.join(", ")
            )
        })
    }
}

/// Output directory for cached compilations, resolved against the root.
fn resolve_out_dir(root: &Path, config: &Config) -> PathBuf {
    let out = &config.solc.out;
    if out.is_absolute() {
        out.clone()
    } else {
        root.join(out)
    }
}
