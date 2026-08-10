//! Corpus types and shared state for the Ripfuzz fuzzer.
//!
//! [`SharedCorpus`] is responsible for:
//! - Loading and validating corpus from disk.
//! - Defining [`Item`] which is convertible to/from
//!   [`evm::Transaction`](crate::evm::Transaction).
//! - Serializing corpus items as compact JSON.
//! - Providing [`next_item`](SharedCorpus::next_item) to return a randomly
//!   selected corpus item (mutated when sourced from the existing pool).
//! - Providing [`add_item`](SharedCorpus::add_item) to add interesting sequences
//!   to the collection.

pub use call::Call;
pub use config::CorpusConfig;
pub use extractor::ExtractedLiterals;
pub use failed_item::SharedFailedCorpusItem;
pub use item::Item;
pub use random::{RandomDynSolValue, random_uint};

pub use replayer::CorpusReplayer;
pub use shared::{CorpusStats, SharedCorpus, Stats};

mod call;
mod config;
mod extractor;
mod failed_item;
mod item;
mod random;
mod replayer;
mod shared;
