//! A single item in the fuzzing corpus.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fuzzer::corpus::Call;

/// A single item in the fuzzing corpus.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Item {
    pub calls: Vec<Call>,
}

impl Item {
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
}

impl From<Vec<Call>> for Item {
    fn from(calls: Vec<Call>) -> Self {
        Self { calls }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn item_id_is_unique_for_different_calls() {
        let item1 = Item::from(vec![Call {
            selector: [0x12, 0x34, 0x56, 0x78],
            args: vec![0u8; 32],
            ..Default::default()
        }]);
        let item2 = Item::from(vec![Call {
            selector: [0xab, 0xcd, 0xef, 0x01],
            args: vec![0u8; 32],
            ..Default::default()
        }]);
        assert_ne!(item1.id(), item2.id());
    }

    #[test]
    fn item_path_is_correct() {
        let item = Item::from(vec![Call {
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
