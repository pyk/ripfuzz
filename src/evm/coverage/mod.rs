//! Coverage collection for the EVM chain abstraction.

pub use context::CoverageContext;
pub use exec::{ExecutionContractCoverage, ExecutionCoverage};
pub use inspector::Inspector;
pub use report::CoverageReporter;
pub use shared::{CoverageUpdate, SharedCoverage};

mod context;
mod edge;
mod exec;
mod inspector;
mod report;
mod shared;
mod source_map;
