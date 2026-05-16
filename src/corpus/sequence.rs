//! Call sequence input type for LibAFL.

use std::hash::{Hash, Hasher};
use std::io::Read;

use libafl::corpus::CorpusId;
use libafl::inputs::{HasTargetBytes, Input};
use libafl_bolts::ownedref::OwnedSlice;
use serde::{Deserialize, Serialize};

use crate::corpus::Call;

/// A sequence of calls targeting a deployed contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallSequenceInput {
    pub calls: Vec<Call>,
}

impl CallSequenceInput {
    /// Create an empty sequence.
    pub fn new() -> Self {
        Self { calls: Vec::new() }
    }

    /// Create a sequence from a single call.
    pub fn single(call: Call) -> Self {
        Self { calls: vec![call] }
    }

    /// Flatten the entire sequence into a single byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.calls.iter().map(|c| c.encoded_size()).sum());
        for call in &self.calls {
            buf.extend_from_slice(&call.selector);
            buf.extend_from_slice(&call.args);
        }
        buf
    }
}

impl Default for CallSequenceInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Input for CallSequenceInput {
    // checkrs: allow(path_param_types)
    fn to_file<P>(&self, path: P) -> Result<(), libafl_bolts::Error>
    where
        P: AsRef<std::path::Path>,
    {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| libafl_bolts::Error::serialize(format!("json failed: {e}")))?;
        libafl_bolts::fs::write_file_atomic(path, &bytes)
    }

    // checkrs: allow(path_param_types)
    fn from_file<P>(path: P) -> Result<Self, libafl_bolts::Error>
    where
        P: AsRef<std::path::Path>,
    {
        let mut file = std::fs::File::open(path)?;
        let mut bytes = vec![];
        file.read_to_end(&mut bytes)?;
        let input = serde_json::from_slice(&bytes)
            .map_err(|e| libafl_bolts::Error::serialize(format!("json parse failed: {e}")))?;
        Ok(input)
    }

    fn generate_name(&self, _id: Option<CorpusId>) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let uuid = uuid::Uuid::new_v4();
        format!("{ts}-{uuid}.json")
    }
}

impl HasTargetBytes for CallSequenceInput {
    fn target_bytes(&self) -> OwnedSlice<'_, u8> {
        OwnedSlice::from(self.to_bytes())
    }
}

// Manual Hash impl because we want to hash the flat bytes.
impl Hash for CallSequenceInput {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_bytes().hash(state);
    }
}

impl PartialEq for CallSequenceInput {
    fn eq(&self, other: &Self) -> bool {
        self.to_bytes() == other.to_bytes()
    }
}

impl Eq for CallSequenceInput {}
