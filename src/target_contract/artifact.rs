use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::JsonAbi;
use revm::bytecode::Bytecode;
use revm::primitives::{Address, Bytes};
use serde::Deserialize;

use crate::target_contract::contract::TargetContract;

/// The subset of a Foundry artifact JSON that Raptor needs.
#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactJson {
    pub abi: JsonAbi,
    pub bytecode: ArtifactBytecode,
    #[serde(rename = "deployedBytecode")]
    pub deployed_bytecode: ArtifactBytecode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactBytecode {
    #[serde(default)]
    pub object: String,
    #[serde(default, rename = "sourceMap")]
    pub source_map: String,
}

impl ArtifactJson {
    /// Build a [`TargetContract`] from this artifact and the deployment
    /// [`Address`].
    pub fn into_target(self, deployed_address: Address) -> TargetContract {
        let initcode = parse_hex(&self.bytecode.object).unwrap_or_default();
        let runtime = parse_hex(&self.deployed_bytecode.object).unwrap_or_default();

        TargetContract {
            deployed_address,
            initcode,
            runtime: Bytecode::new_raw(runtime),
            abi: self.abi,
            properties: vec![],
        }
    }
}

fn parse_hex(s: &str) -> Option<Bytes> {
    let s = s.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).ok().map(Into::into)
}

/// Create [`DynSolValue`] arguments from ABI-encoded hex.
pub fn decode_args(abi: &JsonAbi, name: &str, data: &str) -> Vec<DynSolValue> {
    let func = match abi.function(name).and_then(|v| v.first()) {
        Some(f) => f,
        None => return vec![],
    };

    let data = match parse_hex(data) {
        Some(d) => d,
        None => return vec![],
    };

    let selector = func.selector();
    if data.len() < 4 || &data[..4] != selector.as_slice() {
        return vec![];
    }

    let types: Vec<DynSolType> = match func
        .inputs
        .iter()
        .map(|p| p.selector_type().parse::<DynSolType>())
        .collect::<Result<_, _>>()
    {
        Ok(t) => t,
        Err(_) => return vec![],
    };

    let tuple = DynSolType::Tuple(types);
    match tuple.abi_decode_params(&data[4..]) {
        Ok(DynSolValue::Tuple(values)) => values,
        Ok(other) => vec![other],
        Err(_) => vec![],
    }
}
