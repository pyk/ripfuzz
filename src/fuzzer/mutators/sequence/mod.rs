//! Mutators that modify call sequences by inserting, deleting, or swapping calls.

pub use delete::SequenceDeleteMutator;
pub use insert::SequenceInsertMutator;
pub use swap::SequenceSwapMutator;
mod delete;
mod insert;
mod swap;
