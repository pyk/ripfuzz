//! Contract artifact structure and invariant discovery.

use std::collections::HashMap;
use std::path::PathBuf;

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::{Function, JsonAbi, StateMutability};
use anyhow::{Result, ensure};
use revm::bytecode::Bytecode;
use revm::primitives::Bytes;

use crate::contract::source_map::SourceMap;
use crate::foundry::build_artifact::BuildArtifact;
use crate::foundry::build_artifact::BuildArtifactId;
use crate::foundry::build_artifact::ContractArtifact as FoundryContractArtifact;

/// A Foundry-compiled artifact loaded from disk.
#[derive(Debug, Clone)]
pub struct ContractArtifact {
    pub contract_name: String,
    pub initcode: Bytes,
    pub runtime: Bytecode,
    pub abi: JsonAbi,
    /// Function selectors that represent invariants (must be pure or view).
    pub invariants: Vec<([u8; 4], String)>,
    /// All contracts compiled in the same project, keyed by initcode.
    /// Each entry holds the contract name and ABI for that contract.
    pub initcode_map: HashMap<Bytes, (String, JsonAbi)>,
    /// Raw source map string for initcode, parsed on demand.
    pub init_source_map_raw: Option<String>,
    /// Raw source map string for runtime bytecode, parsed on demand.
    pub runtime_source_map_raw: Option<String>,
    /// Source file path used for source map resolution.
    pub source_path: PathBuf,
}

impl ContractArtifact {
    /// ABI functions the fuzzer will call to mutate state (everything that is
    /// not an invariant).
    pub fn target_functions(&self) -> impl Iterator<Item = &Function> + '_ {
        self.abi
            .functions()
            .filter(|f| !f.name.starts_with("invariant_"))
    }

    /// Parse the init source map on demand.
    pub fn init_source_map(&self) -> Option<SourceMap> {
        self.init_source_map_raw.as_ref().map(|raw| {
            let mut map = SourceMap::parse(raw);
            map.contract_name = self.contract_name.clone();
            map.source_path = self.source_path.clone();
            map
        })
    }

    /// Parse the runtime source map on demand.
    pub fn runtime_source_map(&self) -> Option<SourceMap> {
        self.runtime_source_map_raw.as_ref().map(|raw| {
            let mut map = SourceMap::parse(raw);
            map.contract_name = self.contract_name.clone();
            map.source_path = self.source_path.clone();
            map
        })
    }

    /// Build a [`ContractArtifact`] from a Foundry project artifact and the
    /// full set of compiled build artifacts.
    ///
    /// Bytecode is parsed immediately because it is required for deployment and
    /// execution. Source maps are kept as raw strings and only parsed on
    /// demand via [`Self::init_source_map`] and [`Self::runtime_source_map`].
    pub fn from_foundry_artifact(
        target: &FoundryContractArtifact,
        all_artifacts: &HashMap<BuildArtifactId, BuildArtifact>,
        project_root: impl AsRef<std::path::Path>,
    ) -> Result<Self> {
        let initcode =
            crate::foundry::artifact::parse_hex(&target.bytecode.object).unwrap_or_default();
        let runtime = crate::foundry::artifact::parse_hex(&target.deployed_bytecode.object)
            .unwrap_or_default();

        let mut initcode_map = HashMap::new();
        let entries: Vec<(Bytes, (String, JsonAbi))> = all_artifacts
            .values()
            .filter_map(|artifact| {
                let (bytecode, name, abi) = match artifact {
                    BuildArtifact::Contract(c) => (&c.bytecode, &c.id.name, &c.abi),
                    BuildArtifact::Library(c) => (&c.bytecode, &c.id.name, &c.abi),
                    _ => return None,
                };
                let code =
                    crate::foundry::artifact::parse_hex(&bytecode.object).unwrap_or_default();
                if code.is_empty() {
                    return None;
                }
                Some((code, (name.clone(), abi.clone())))
            })
            .collect();
        initcode_map.extend(entries);

        let invariants = find_and_validate_invariants(&target.abi)?;

        Ok(Self {
            contract_name: target.id.name.clone(),
            initcode,
            runtime: Bytecode::new_raw(runtime),
            abi: target.abi.clone(),
            invariants,
            initcode_map,
            init_source_map_raw: if target.bytecode.source_map.is_empty() {
                None
            } else {
                Some(target.bytecode.source_map.clone())
            },
            runtime_source_map_raw: if target.deployed_bytecode.source_map.is_empty() {
                None
            } else {
                Some(target.deployed_bytecode.source_map.clone())
            },
            source_path: project_root.as_ref().join(&target.id.path),
        })
    }
}

/// Scan the ABI for functions that start with `invariant_` and validate
/// that every one of them is either `pure` or `view`.
pub fn find_and_validate_invariants(abi: &JsonAbi) -> Result<Vec<([u8; 4], String)>> {
    let mut invariants = Vec::new();

    for func in abi.functions() {
        if !func.name.starts_with("invariant_") {
            continue;
        }
        ensure!(
            matches!(
                func.state_mutability,
                StateMutability::Pure | StateMutability::View
            ),
            "invariant function '{}' must be declared pure or view",
            func.name
        );
        let sel: [u8; 4] = func.selector().into();
        invariants.push((sel, func.name.to_owned()));
    }

    Ok(invariants)
}

/// ABI-encode a function call given its ABI and human-readable arguments.
pub fn encode_call(abi: &JsonAbi, name: &str, args: &[DynSolValue]) -> Option<Bytes> {
    let func = abi.function(name)?.first()?;
    let mut buf = Vec::new();
    buf.extend_from_slice(func.selector().as_slice());
    let encoded = DynSolValue::Tuple(args.to_vec()).abi_encode_params();
    buf.extend_from_slice(&encoded);
    Some(buf.into())
}
