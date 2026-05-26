//! A single item in the fuzzing corpus.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use revm::primitives::Bytes;
use serde::{Deserialize, Serialize};

use crate::evm::chain::{ExecInput, Transaction};
use crate::fuzzer::corpus::Call;

/// A single item in the fuzzing corpus.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Item {
    pub calls: Vec<Call>,
    pub weight: u64,
    #[serde(default)]
    pub total_mutations: u64,
    #[serde(default)]
    pub new_finds_produced: u64,
    #[serde(skip, default)]
    pub(crate) is_replay: bool,
}

impl Item {
    pub fn new(calls: Vec<Call>) -> Self {
        Self {
            calls,
            weight: 1,
            total_mutations: 0,
            new_finds_produced: 0,
            is_replay: false,
        }
    }

    /// Unique identifier derived from the call sequence.
    pub fn id(&self) -> String {
        let mut hasher = DefaultHasher::new();
        self.calls.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// On-disk path for this corpus item.
    pub fn path(
        &self,
        corpus_dir: impl AsRef<Path>,
        artifact_id: &crate::foundry::ArtifactId,
    ) -> PathBuf {
        corpus_dir
            .as_ref()
            .join(&artifact_id.path)
            .join(&artifact_id.name)
            .join(format!("{}.json", self.id()))
    }

    /// Convert this corpus item into an [`ExecInput`] for the given caller
    /// and target address.
    pub fn into_exec_input(
        self,
        caller: alloy_primitives::Address,
        target: alloy_primitives::Address,
    ) -> crate::evm::chain::ExecInput {
        ExecInput::new(
            self.calls
                .into_iter()
                .map(|call| {
                    Transaction::new(target)
                        .caller(caller)
                        .calldata(Bytes::from(call.encode()))
                })
                .collect(),
        )
    }
}

impl From<Vec<Call>> for Item {
    fn from(calls: Vec<Call>) -> Self {
        Self::new(calls)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn item_id_is_unique_for_different_calls() {
        let item1 = Item::new(vec![Call {
            selector: [0x12, 0x34, 0x56, 0x78],
            args: vec![0u8; 32],
            ..Default::default()
        }]);
        let item2 = Item::new(vec![Call {
            selector: [0xab, 0xcd, 0xef, 0x01],
            args: vec![0u8; 32],
            ..Default::default()
        }]);
        assert_ne!(item1.id(), item2.id());
    }

    #[test]
    fn item_path_is_correct() {
        let item = Item::new(vec![Call {
            selector: [0x12, 0x34, 0x56, 0x78],
            args: vec![0u8; 32],
            ..Default::default()
        }]);
        let artifact_id = crate::foundry::ArtifactId {
            path: PathBuf::from("src/Counter.sol"),
            name: "Counter".into(),
        };
        let path = item.path("/tmp/corpus", &artifact_id);
        let expected = PathBuf::from(format!(
            "/tmp/corpus/src/Counter.sol/Counter/{}.json",
            item.id()
        ));
        assert_eq!(path, expected);
    }
}
