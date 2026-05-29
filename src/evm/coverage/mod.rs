//! Coverage collection for the EVM chain abstraction.

pub use exec::{ExecutionContractCoverage, ExecutionCoverage};
pub use inspector::Inspector;
pub use shared::{ContractCoverage, CoverageUpdate, SharedCoverage};

mod edge;
mod exec;
mod inspector;
mod shared;
