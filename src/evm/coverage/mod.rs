//! Coverage collection for the EVM chain abstraction.

pub use exec::ExecutionCoverage;
pub use inspector::Inspector;
pub use shared::SharedCoverage;

mod edge;
mod exec;
mod inspector;
mod shared;
