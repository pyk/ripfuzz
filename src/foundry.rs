use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct FoundryArtifact {
    pub bytecode: BytecodeField,
    #[serde(rename = "methodIdentifiers")]
    pub method_identifiers: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BytecodeField {
    pub object: String,
}

impl FoundryArtifact {
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let artifact = serde_json::from_str(&content)?;
        Ok(artifact)
    }

    pub fn creation_bytecode(&self) -> anyhow::Result<Vec<u8>> {
        decode_hex(&self.bytecode.object)
    }
}

fn decode_hex(s: &str) -> anyhow::Result<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    Ok(hex::decode(s)?)
}
