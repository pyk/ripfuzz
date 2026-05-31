//! Coverage collection for the EVM chain abstraction.

pub use exec::{ExecutionContractCoverage, ExecutionCoverage};
pub use inspector::Inspector;
pub use report::{CoverageReport, SourceFile};
pub use shared::{CoverageUpdate, SharedCoverage};

mod edge;
mod exec;
mod inspector;
mod report;
mod shared;
mod source_map;
