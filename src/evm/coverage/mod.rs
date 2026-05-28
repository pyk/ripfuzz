//! Coverage collection for the EVM chain abstraction.

pub use edge::{DEPTH_TRACKED_PCS, edge_marker};
pub use exec::{ExecutionContractCoverage, ExecutionCoverage};
pub use inspector::Inspector;
pub use shared::{ContractCoverage, CoverageUpdate, SharedCoverage};

pub mod edge;
pub mod exec;
pub mod inspector;
pub mod shared;
