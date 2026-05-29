//! Execution output type.

use crate::evm::{ExecutionCoverage, result, trace};

/// Result of executing a sequence of transactions.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub results: Vec<result::TransactionResult>,
    pub trace: Option<trace::Trace>,
    pub coverage: Option<ExecutionCoverage>,
    pub panic_transactions: Vec<super::Transaction>,
}
