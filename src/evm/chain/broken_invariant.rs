//! Broken invariant reports decoded from a dedicated custom error revert.
//!
//! A harness reports a broken invariant by reverting with the
//! `BrokenInvariantError` custom error, for example:
//!
//! ```solidity
//! error BrokenInvariantError(string id, string description);
//!
//! function invariant_total_below_limit() external view {
//!     if (total > 100) {
//!         revert BrokenInvariantError({id: "INV-001", description: "total exceeded 100"});
//!     }
//! }
//! ```

use alloy_sol_types::SolError;

alloy_sol_types::sol! {
    error BrokenInvariantError(string id, string description);
}

/// One broken invariant: the id and description carried by a
/// `BrokenInvariantError` revert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokenInvariant {
    pub id: String,
    pub description: String,
}

impl BrokenInvariant {
    /// Decode the report from revert output, returning `None` when the
    /// output is not a `BrokenInvariantError` revert or the id is empty.
    pub fn from_revert(output: &[u8]) -> Option<Self> {
        // 1. Require the custom error selector.
        if output.len() < 4 || output[..4] != BrokenInvariantError::SELECTOR {
            return None;
        }

        // 2. Decode the id and description from the revert payload.
        let error = BrokenInvariantError::abi_decode(output).ok()?;

        // 3. Reject reports without an id, the dedup key must be meaningful.
        if error.id.is_empty() {
            return None;
        }

        Some(Self {
            id: error.id,
            description: error.description,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloy_sol_types::SolValue;
    use revm::primitives::Bytes;

    use super::*;

    fn revert_output(id: &str, description: &str) -> Bytes {
        let mut encoded = BrokenInvariantError::SELECTOR.to_vec();
        encoded.extend((id, description).abi_encode_params());
        Bytes::from(encoded)
    }

    #[test]
    fn from_revert_decodes_id_and_description() {
        let output = revert_output("INV-001", "total exceeded 100");
        let broken = BrokenInvariant::from_revert(&output).unwrap();

        assert_eq!(broken.id, "INV-001");
        assert_eq!(broken.description, "total exceeded 100");
    }

    #[test]
    fn from_revert_rejects_other_selectors() {
        let mut encoded = [0x4e, 0x48, 0x7b, 0x71].to_vec();
        encoded.extend((1u64,).abi_encode_params());

        assert!(BrokenInvariant::from_revert(&encoded).is_none());
        assert!(BrokenInvariant::from_revert(&[]).is_none());
    }

    #[test]
    fn from_revert_rejects_malformed_payload() {
        let mut encoded = BrokenInvariantError::SELECTOR.to_vec();
        encoded.extend([0u8; 8]);

        assert!(BrokenInvariant::from_revert(&encoded).is_none());
    }

    #[test]
    fn from_revert_rejects_empty_id() {
        let output = revert_output("", "details");

        assert!(BrokenInvariant::from_revert(&output).is_none());
    }
}
