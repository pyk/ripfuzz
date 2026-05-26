//! A single item in the fuzzing corpus.

use std::path::{Path, PathBuf};

use alloy_primitives::keccak256;
use serde::{Deserialize, Serialize};

use crate::fuzzer::corpus::Call;

/// A single item in the fuzzing corpus.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Item {
    pub calls: Vec<Call>,
}

impl Item {
    /// Unique identifier derived from the call sequence.
    ///
    /// Computes a Keccak256 hash over the concatenated content hashes of
    /// each call, matching Medusa's `CallSequence.Hash()` approach. This
    /// makes the ID deterministic, stable across restarts, and collision
    /// resistant compared with the previous 64-bit SipHash.
    pub fn id(&self) -> String {
        let mut buf = Vec::with_capacity(self.calls.len() * 32);
        for call in &self.calls {
            buf.extend_from_slice(&call.content_hash());
        }
        let hash = keccak256(&buf);
        hex::encode(hash)
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

    /// Known Keccak256 ID for the fixture
    const STABLE_ID: &str = "4ff511692f34d70433734c4e39052681dc4db935a7871b2ab78e0d185dbb9e6f";

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

    /// Two calls with identical execution data but different human-readable
    /// metadata must hash to the same ID. This is the Medusa-style
    /// content-hash deduplication property.
    #[test]
    fn item_id_ignores_human_readable_metadata() {
        let item_a = Item::from(vec![Call {
            selector: [0x12, 0x34, 0x56, 0x78],
            args: vec![0u8; 32],
            block_number_delay: 2,
            block_timestamp_delay: 3,
            method_name: "foo".into(),
            method_signature: "foo(uint256)".into(),
            input_values: vec![serde_json::json!(42)],
        }]);
        let item_b = Item::from(vec![Call {
            selector: [0x12, 0x34, 0x56, 0x78],
            args: vec![0u8; 32],
            block_number_delay: 2,
            block_timestamp_delay: 3,
            method_name: "bar".into(),
            method_signature: "bar(uint256)".into(),
            input_values: vec![serde_json::json!(99)],
        }]);
        assert_eq!(item_a.id(), item_b.id());
    }

    /// The corpus item ID must remain identical after a full save/load cycle.
    /// This enforces the Medusa-style stability guarantee: the content hash
    /// depends only on execution-relevant fields, not on transient state or
    /// serialization noise.
    #[test]
    fn item_id_is_stable_across_restart() {
        // 1. Build an item in memory with known execution data.
        let item = Item::from(vec![Call {
            selector: [0x12, 0x34, 0x56, 0x78],
            args: vec![0u8; 32],
            block_number_delay: 2,
            block_timestamp_delay: 3,
            method_name: "foo".into(),
            method_signature: "foo(uint256)".into(),
            input_values: vec![serde_json::json!(42)],
        }]);
        let id_before = item.id();
        assert_eq!(
            id_before, STABLE_ID,
            "in-memory item with known data must produce the stable hash"
        );

        // 2. Round-trip through JSON and verify the ID is preserved.
        let json = serde_json::to_string(&item).expect("item must serialize to JSON");
        let item_roundtrip: Item =
            serde_json::from_str(&json).expect("item must deserialize from JSON");
        assert_eq!(
            item_roundtrip.id(),
            STABLE_ID,
            "ID must survive a JSON serialize/deserialize cycle"
        );
    }
}
