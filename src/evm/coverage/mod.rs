//! Coverage collection for the EVM chain abstraction.

pub use edge::{DEPTH_TRACKED_PCS, edge_marker};
pub use inspector::Inspector;
pub use map::{
    ContractCoverage, CoverageMap, CoverageUpdate, LocalContractCoverage, LocalCoverage,
};

pub mod edge;
pub mod inspector;
pub mod map;
