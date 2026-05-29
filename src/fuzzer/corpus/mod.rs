//! Thread-safe corpus shared across parallel fuzzer threads.
//!
//! ## Separation of concerns
//!
//! [`SharedCorpus`] is responsible for:
//! - Loading and validating corpus from disk.
//! - Defining [`Item`] which is convertible to/from
//!   [`evm::Transaction`](crate::evm::Transaction).
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
pub use extractor::ExtractedLiterals;
pub use item::Item;

pub use replayer::CorpusReplayer;
pub use shared::SharedCorpus;

mod call;
mod config;
mod extractor;
mod item;
mod random;
mod replayer;
mod shared;
