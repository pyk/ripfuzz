//! revm-native EVM chain abstraction.
//!
//! `evm::Chain` owns EVM state and executes raw transactions. It does not know
//! about fuzzer concepts (invariants, corpus calls, labels, or contract setup).
//! Those concerns live in the caller.
//!
//! Responsibilities:
//! 1. Own EVM state: [`BlockEnv`](revm::context::BlockEnv),
//!    [`CfgEnv`](revm::context::CfgEnv), and a generic database `D`.
//! 2. Execute raw transactions: [`Chain::deploy`], [`Chain::call`],
//!    [`Chain::transact`].
//! 3. Return pure data structures for traces; never format or print them.

pub use chain::{Chain, DEFAULT_DEPLOYER, ForkConfig};
pub use result::{CallFrame, Trace, TraceInspector, TransactionResult};
pub use specs::get_spec_id;

mod chain;
mod result;
mod specs;
