//! Execution output type.

use crate::evm::chain::Transaction;
use crate::evm::{ExecutionCoverage, result, trace};

/// Result of executing a sequence of transactions.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub results: Vec<result::TransactionResult>,
    pub trace: Option<trace::Trace>,
    pub coverage: Option<ExecutionCoverage>,
    pub panic_transactions: Vec<Transaction>,
}

impl ExecOutput {
    /// Check whether any transaction triggered a failure.
    ///
    /// When `fail_on_revert` is enabled, any reverted transaction is treated as
    /// a failure. Otherwise only `assert` panics are considered failures.
    pub fn has_failure(&self, fail_on_revert: bool) -> bool {
        if fail_on_revert {
            self.results.iter().any(|r| !r.success)
        } else {
            !self.panic_transactions.is_empty()
        }
    }
}
