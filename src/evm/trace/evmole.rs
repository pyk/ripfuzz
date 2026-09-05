//! Analyze deployed EVM bytecode with the evmole crate.
//!
//! [`Evmole`] extracts function selectors and argument types.
//! [`super::TraceContext`] consumes the result when no project ABI matches
//! the call selector.
//!
//! ```no_run
//! use ripfuzz::evm::Evmole;
//! let runtime_code: &[u8] = &[];
//! let extracted = Evmole::extract(runtime_code);
//! let _ = extracted.arguments(&[0u8; 4]);
//! ```

use std::collections::HashMap;

use alloy_dyn_abi::DynSolType;
use alloy_primitives::{B256, keccak256};
use tracing::debug;

/// Function argument types extracted from deployed EVM bytecode by evmole.
pub struct Evmole {
    hash: B256,
    functions: HashMap<[u8; 4], Vec<DynSolType>>,
}

impl Evmole {
    /// Analyze `code` and extract function selectors with argument types.
    pub fn extract(code: &[u8]) -> Self {
        let hash = keccak256(code);
        let info = ::evmole::contract_info(
            ::evmole::ContractInfoArgs::new(code)
                .with_selectors()
                .with_arguments(),
        );
        let mut functions = HashMap::new();
        if let Some(fns) = info.functions {
            for func in fns {
                let args = func.arguments.unwrap_or_default();
                functions.insert(func.selector, args);
            }
        }
        debug!(
            code_hash = %hash,
            functions = functions.len(),
            "extracted evmole abi"
        );
        Self { hash, functions }
    }

    /// Keccak-256 of the bytecode this result was extracted from.
    pub fn hash(&self) -> B256 {
        self.hash
    }

    /// Argument types for `selector`, if this bytecode dispatches it.
    pub fn arguments(&self, selector: &[u8; 4]) -> Option<&[DynSolType]> {
        self.functions.get(selector).map(|v| v.as_slice())
    }
}
