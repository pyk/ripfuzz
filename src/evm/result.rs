//! Transaction result types for the EVM chain.

use std::time::Duration;

use revm::context_interface::result::{ExecutionResult, Output};
use revm::primitives::{Address, Bytes};

use crate::evm::forkdb::RpcStats;

/// Solidity `Panic(uint256)` selector: keccak256("Panic(uint256)")[:4]
const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];

/// Result of a single EVM transaction execution.
#[derive(Debug, Clone, Default)]
pub struct TransactionResult {
    pub success: bool,
    pub gas_used: u64,
    pub output: Option<Bytes>,
    pub logs: Vec<revm::primitives::Log>,
    pub created_address: Option<Address>,
    /// Wall time spent executing this transaction, including RPC waits.
    pub elapsed: Duration,
    /// RPC cache hits/misses attributed to this transaction's thread.
    pub rpc: RpcStats,
}

impl From<ExecutionResult> for TransactionResult {
    fn from(result: ExecutionResult) -> Self {
        match result {
            ExecutionResult::Success {
                gas, logs, output, ..
            } => {
                let (out, addr) = match output {
                    Output::Call(b) => (Some(b), None),
                    Output::Create(b, addr) => (Some(b), addr),
                };
                Self {
                    success: true,
                    gas_used: gas.tx_gas_used(),
                    output: out,
                    logs,
                    created_address: addr,
                    elapsed: Duration::ZERO,
                    rpc: RpcStats::default(),
                }
            }
            ExecutionResult::Revert {
                gas, logs, output, ..
            } => Self {
                success: false,
                gas_used: gas.tx_gas_used(),
                output: Some(output),
                logs,
                created_address: None,
                elapsed: Duration::ZERO,
                rpc: RpcStats::default(),
            },
            ExecutionResult::Halt { gas, logs, .. } => Self {
                success: false,
                gas_used: gas.tx_gas_used(),
                output: None,
                logs,
                created_address: None,
                elapsed: Duration::ZERO,
                rpc: RpcStats::default(),
            },
        }
    }
}

impl TransactionResult {
    /// Detect a Solidity `assert` failure (`Panic(0x01)`) in revert output.
    pub fn is_assert_failure(&self) -> bool {
        match &self.output {
            Some(output) => {
                output.len() >= 36 && output[..4] == PANIC_SELECTOR && output[35] == 0x01
            }
            None => false,
        }
    }
}
