//! Coverage collection for the EVM chain abstraction.

pub use exec::{ExecutionContractCoverage, ExecutionCoverage};
pub use id::CoverageId;
pub use inspector::Inspector;
pub use reporter::{CoverageReport, CoverageReporter};
pub use shared::{CoverageUpdate, SharedCoverage};

mod edge;
mod exec;
mod id;
mod inspector;
mod reporter;
mod shared;
mod source_map;
