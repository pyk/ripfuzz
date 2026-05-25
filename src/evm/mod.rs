//! revm-native EVM chain abstraction.
//!
//! `evm::Chain` owns EVM state and executes raw transactions. It does not know
//! about fuzzer concepts (invariants, corpus calls, labels, or contract setup).
//! Those concerns live in the caller.
//!
//! Responsibilities:
//! 1. Own EVM state: [`BlockEnv`](revm::context::BlockEnv),
//!    [`CfgEnv`](revm::context::CfgEnv), and a [`Database`].
//! 2. Execute raw transactions: [`Chain::deploy`], [`Chain::call`],
//!    [`Chain::transact`].
//! 3. Return pure data structures for traces; never format or print them.

pub use chain::{
    Chain, DEFAULT_DEPLOYER, DeployInput, DeployOutput, ExecInput, ExecOutput, SetupInput,
    SetupOutput, Transaction,
};
pub use database::{Database, DatabaseError};
pub use result::TransactionResult;
pub use specs::get_spec_id;
pub use trace::{CallFrame, Inspector, Trace};

pub mod chain;
pub mod cheatcode;
pub mod coverage;
pub mod database;
pub mod forkdb;
pub mod result;
pub mod specs;
pub mod trace;
