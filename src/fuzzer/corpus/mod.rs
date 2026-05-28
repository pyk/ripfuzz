//! Thread-safe corpus shared across parallel fuzzer threads.
//!
//! ## Separation of concerns
//!
//! [`SharedCorpus`] is responsible for:
//! - Loading and validating corpus from disk.
//! - Defining [`Item`] which is convertible to/from
//!   [`evm::chain::ExecInput`](crate::evm::chain::ExecInput).
//! - Serializing corpus items as compact JSON.
//! - Providing [`next_item`](SharedCorpus::next_item) to return a randomly
//!   selected corpus item (mutated when sourced from the existing pool) for a
//!   fuzzer thread.
//! - Providing [`add_item`](SharedCorpus::add_item) to add interesting sequences
//!   to the collection.
//!
//! [`Fuzzer`](crate::fuzzer::Fuzzer) is responsible for:
//! - Using [`next_item`](SharedCorpus::next_item) to obtain the next input to
//!   execute.
//! - Using [`add_item`](SharedCorpus::add_item) to store interesting sequences
//!   discovered during execution.

pub use call::Call;
pub use config::Config;
pub use extractor::{ExtractedLiterals, extract_literals};
pub use item::Item;

pub use shared::SharedCorpus;
pub use shared::get_dir;

pub mod call;
pub mod config;
pub mod extractor;
pub mod item;
pub mod random;
pub mod replayer;
pub mod shared;
