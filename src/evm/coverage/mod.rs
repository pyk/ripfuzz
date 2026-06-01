//! Coverage collection for the EVM chain abstraction.

pub use context::CoverageContext;
pub use exec::{ExecutionContractCoverage, ExecutionCoverage};
pub use inspector::Inspector;
pub use reporter::CoverageReporter;
pub use shared::{CoverageUpdate, SharedCoverage};

mod context;
mod edge;
mod exec;
mod inspector;
mod reporter;
mod shared;
mod source_map;
