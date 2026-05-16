//! Foundry artifact JSON parsing and contract metadata extraction.

use std::collections::HashMap;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_json_abi::JsonAbi;
use revm::bytecode::Bytecode;
use revm::primitives::Bytes;
use serde::Deserialize;

use crate::contract::artifact;

/// The subset of a Foundry artifact JSON that Raptor needs.
#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactJson {
    pub abi: JsonAbi,
    pub bytecode: ArtifactBytecode,
    #[serde(rename = "deployedBytecode")]
    pub deployed_bytecode: ArtifactBytecode,
    #[serde(default, rename = "metadata")]
    pub metadata: Option<ArtifactMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactMetadata {
    pub settings: Option<ArtifactSettings>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactSettings {
    #[serde(rename = "compilationTarget")]
    pub compilation_target: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactBytecode {
    #[serde(default)]
    pub object: String,
    #[serde(default, rename = "sourceMap")]
    pub source_map: String,
}

impl ArtifactJson {
    /// Build a [`artifact::ContractArtifact`] from this artifact.
    pub fn into_artifact(self, contract_name: &str) -> artifact::ContractArtifact {
        let initcode = parse_hex(&self.bytecode.object).unwrap_or_default();
        let runtime = parse_hex(&self.deployed_bytecode.object).unwrap_or_default();

        artifact::ContractArtifact {
            contract_name: contract_name.to_owned(),
            initcode,
            runtime: Bytecode::new_raw(runtime),
            abi: self.abi,
            properties: vec![],
            initcode_map: HashMap::new(),
        }
    }

    /// Build a [`artifact::ContractArtifact`] from this artifact with all project contracts.
    pub fn into_artifact_with_all(
        self,
        contract_name: &str,
        all_contracts: HashMap<String, (Bytes, JsonAbi)>,
    ) -> crate::contract::artifact::ContractArtifact {
        let initcode = parse_hex(&self.bytecode.object).unwrap_or_default();
        let runtime = parse_hex(&self.deployed_bytecode.object).unwrap_or_default();
        let initcode_map: HashMap<Bytes, (String, JsonAbi)> = all_contracts
            .into_iter()
            .map(|(name, (initcode, abi))| (initcode, (name, abi)))
            .collect();

        crate::contract::artifact::ContractArtifact {
            contract_name: contract_name.to_owned(),
            initcode,
            runtime: Bytecode::new_raw(runtime),
            abi: self.abi,
            properties: vec![],
            initcode_map,
        }
    }
}

pub fn parse_hex(s: &str) -> Option<Bytes> {
    let s = s.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).map_or(None, |v| Some(v.into()))
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
        .collect()
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
