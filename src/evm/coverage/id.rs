//! Coverage identifier keyed by address for runtime and hash for initcode.

use alloy_primitives::{Address, B256};

/// Identifier for a contract's coverage map.
///
/// For initcode (constructor) the identifier is the code hash. For runtime
/// bytecode it is the pair `(address, codehash)` so that two factory clones
/// with identical bytecode but different addresses get distinct coverage maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum CoverageId {
    Initcode(B256),
    Runtime { address: Address, codehash: B256 },
}

impl CoverageId {
    pub fn codehash(&self) -> B256 {
        match self {
            Self::Initcode(h) => *h,
            Self::Runtime { codehash, .. } => *codehash,
        }
    }

    pub fn address(&self) -> Option<Address> {
        match self {
            Self::Initcode(_) => None,
            Self::Runtime { address, .. } => Some(*address),
        }
    }

    pub fn is_initcode(&self) -> bool {
        matches!(self, Self::Initcode(_))
    }
}
