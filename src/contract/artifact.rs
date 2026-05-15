use std::collections::HashMap;

use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::{JsonAbi, StateMutability};
use anyhow::Result;
use revm::bytecode::Bytecode;
use revm::primitives::Bytes;

/// A Foundry-compiled artifact loaded from disk.
#[derive(Debug, Clone)]
pub struct ContractArtifact {
    pub contract_name: String,
    pub initcode: Bytes,
    pub runtime: Bytecode,
    pub abi: JsonAbi,
    /// Function selectors that return `bool` and represent invariants.
    pub properties: Vec<([u8; 4], String)>,
    /// All contracts compiled in the same project, keyed by contract name.
    /// Each entry holds the initcode and ABI for that contract.
    pub all_contracts: HashMap<String, (Bytes, JsonAbi)>,
}

/// Scan the ABI for functions that start with `property_` and validate
/// that every one of them is either `pure` or `view` and returns a single `bool`.
pub fn find_and_validate_properties(abi: &JsonAbi) -> Result<Vec<([u8; 4], String)>> {
    let mut properties = Vec::new();

    for func in abi.functions() {
        if !func.name.starts_with("property_") {
            continue;
        }
        if func.outputs.len() != 1 || func.outputs[0].ty != "bool" {
            anyhow::bail!(
                "property function '{}' must return a single bool",
                func.name
            );
        }
        if !matches!(
            func.state_mutability,
            StateMutability::Pure | StateMutability::View
        ) {
            anyhow::bail!(
                "property function '{}' must be declared pure or view",
                func.name
            );
        }
        let sel: [u8; 4] = func.selector().into();
        properties.push((sel, func.name.clone()));
    }

    Ok(properties)
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
