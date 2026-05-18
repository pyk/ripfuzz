//! Contract artifact structure and invariant discovery.

use std::collections::HashMap;

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::{JsonAbi, StateMutability};
use anyhow::{Result, ensure};
use revm::bytecode::Bytecode;
use revm::primitives::Bytes;

use crate::contract::source_map::SourceMap;

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
    /// Parsed source map for initcode, if present in the artifact.
    pub init_source_map: Option<SourceMap>,
    /// Parsed source map for runtime bytecode, if present in the artifact.
    pub runtime_source_map: Option<SourceMap>,
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
