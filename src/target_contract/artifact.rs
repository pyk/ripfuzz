use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::{JsonAbi, StateMutability};
use revm::bytecode::Bytecode;
use revm::primitives::Bytes;

/// A Foundry-compiled artifact loaded from disk.
#[derive(Debug, Clone)]
pub struct TargetContractArtifact {
    pub contract_name: String,
    pub initcode: Bytes,
    pub runtime: Bytecode,
    pub abi: JsonAbi,
    /// Function selectors that return `bool` and represent invariants.
    pub properties: Vec<([u8; 4], String)>,
}

/// Scan the ABI for functions that:
///   1. Return a single `bool`, and
///   2. Are either `pure` or `view`.
pub fn discover_properties(abi: &JsonAbi) -> Vec<([u8; 4], String)> {
    abi.functions()
        .filter(|f| {
            f.outputs.len() == 1
                && f.outputs[0].ty == "bool"
                && matches!(
                    f.state_mutability,
                    StateMutability::Pure | StateMutability::View
                )
        })
        .map(|f| {
            let sel: [u8; 4] = f.selector().into();
            let name = f.name.clone();
            (sel, name)
        })
        .collect()
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
