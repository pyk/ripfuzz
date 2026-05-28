//! Coverage collection for the EVM chain abstraction.

pub use edge::{DEPTH_TRACKED_PCS, edge_marker};
pub use inspector::Inspector;
pub use local::{LocalContractCoverage, LocalCoverage};
pub use shared::{ContractCoverage, CoverageUpdate, SharedCoverage};

pub mod edge;
pub mod inspector;
pub mod local;
pub mod shared;
