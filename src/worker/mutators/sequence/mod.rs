//! Mutators that modify call sequences by inserting, deleting, swapping, or delaying calls.

pub use delay::SequenceDelayMutator;
pub use delete::SequenceDeleteMutator;
pub use insert::SequenceInsertMutator;
pub use swap::SequenceSwapMutator;
mod delay;
mod delete;
mod insert;
mod swap;
